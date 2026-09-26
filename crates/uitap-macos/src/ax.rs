//! 辅助功能（AX）元素树：读取、按路径定位、执行动作。
//!
//! 元素用「从应用根到目标的子索引链」标识，不保存跨调用的句柄。因此每次操作都要
//! 从根重新走一遍，代价是一次浅层遍历，换来的是无状态、可重复、不泄漏。
//!
//! 走的是 AX API，用的是「辅助功能」授权，与输入合成共用同一项权限。

use std::collections::VecDeque;
use std::ffi::c_void;

use uitap_core::backend::{
    Backend as _, BackendError, ElementAction, ElementNode, Result, TreeLimits, join_path,
};
use uitap_core::geom::Rect;

use crate::sys::*;

/// AX 属性名。这些在头文件里是 `#define CFSTR("...")`，没有可链接符号，只能自己构造。
/// 整棵树共用一份，遍历时不再反复构造。
struct Attrs {
    role: CFStringRef,
    subrole: CFStringRef,
    title: CFStringRef,
    description: CFStringRef,
    value: CFStringRef,
    identifier: CFStringRef,
    position: CFStringRef,
    size: CFStringRef,
    enabled: CFStringRef,
    focused: CFStringRef,
    children: CFStringRef,
    windows: CFStringRef,
}

impl Attrs {
    fn new() -> Self {
        unsafe {
            Self {
                role: cfstring("AXRole"),
                subrole: cfstring("AXSubrole"),
                title: cfstring("AXTitle"),
                description: cfstring("AXDescription"),
                value: cfstring("AXValue"),
                identifier: cfstring("AXIdentifier"),
                position: cfstring("AXPosition"),
                size: cfstring("AXSize"),
                enabled: cfstring("AXEnabled"),
                focused: cfstring("AXFocused"),
                children: cfstring("AXChildren"),
                windows: cfstring("AXWindows"),
            }
        }
    }
}

impl Drop for Attrs {
    fn drop(&mut self) {
        unsafe {
            for key in [
                self.role,
                self.subrole,
                self.title,
                self.description,
                self.value,
                self.identifier,
                self.position,
                self.size,
                self.enabled,
                self.focused,
                self.children,
                self.windows,
            ] {
                CFRelease(key as CFTypeRef);
            }
        }
    }
}

/// 持有的 AXUIElement，作用域结束时释放。
struct AxElement(AXUIElementRef);

impl AxElement {
    fn new(raw: AXUIElementRef) -> Option<Self> {
        if raw.is_null() {
            None
        } else {
            Some(AxElement(raw))
        }
    }

    /// 从 CFTypeRef 接管一个 +1 引用。
    unsafe fn adopt(raw: CFTypeRef) -> Option<Self> {
        AxElement::new(raw as AXUIElementRef)
    }

    fn raw(&self) -> AXUIElementRef {
        self.0
    }

    /// 取属性，返回 +1 引用，由调用方释放。
    fn copy_attribute(&self, name: CFStringRef) -> Option<CFTypeRef> {
        let mut out: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(self.0, name, &mut out) };
        if status != 0 || out.is_null() {
            None
        } else {
            Some(out)
        }
    }

    fn string_attribute(&self, name: CFStringRef) -> Option<String> {
        let value = self.copy_attribute(name)?;
        let text = unsafe {
            let result = if is_string(value) {
                string_value(value as CFStringRef)
            } else {
                None
            };
            CFRelease(value);
            result
        };
        text.filter(|s| !s.is_empty())
    }

    fn bool_attribute(&self, name: CFStringRef) -> Option<bool> {
        let value = self.copy_attribute(name)?;
        let result = unsafe {
            let out = if CFGetTypeID(value) == CFBooleanGetTypeID() {
                Some(CFBooleanGetValue(value as CFBooleanRef))
            } else {
                None
            };
            CFRelease(value);
            out
        };
        result
    }

    fn children(&self, name: CFStringRef) -> Vec<AxElement> {
        let Some(value) = self.copy_attribute(name) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        unsafe {
            if CFGetTypeID(value) == CFArrayGetTypeID() {
                let array = value as CFArrayRef;
                let count = CFArrayGetCount(array);
                for index in 0..count {
                    let item = CFArrayGetValueAtIndex(array, index);
                    if item.is_null() {
                        continue;
                    }
                    // 数组里的元素是借用引用，需要 retain 才能在数组释放后继续持有。
                    let retained = CFRetain(item);
                    if let Some(element) = AxElement::adopt(retained) {
                        out.push(element);
                    }
                }
            }
            CFRelease(value);
        }
        out
    }

    /// 位置与尺寸合成屏幕矩形。两者缺一就不报 bounds。
    fn bounds(&self, attrs: &Attrs) -> Option<Rect> {
        let position = self.point_from(attrs.position)?;
        let size = self.size_from(attrs.size)?;
        Some(Rect::new(position.0, position.1, size.0, size.1))
    }

    fn point_from(&self, name: CFStringRef) -> Option<(f64, f64)> {
        let value = self.copy_attribute(name)?;
        let result = unsafe {
            let out = read_ax_point(value);
            CFRelease(value);
            out
        };
        result
    }

    fn size_from(&self, name: CFStringRef) -> Option<(f64, f64)> {
        let value = self.copy_attribute(name)?;
        let result = unsafe {
            let out = read_ax_size(value);
            CFRelease(value);
            out
        };
        result
    }

    fn value_text(&self, attrs: &Attrs) -> Option<String> {
        let value = self.copy_attribute(attrs.value)?;
        let text = unsafe {
            let out = if is_string(value) {
                string_value(value as CFStringRef)
            } else if CFGetTypeID(value) == CFNumberGetTypeID() {
                number_f64(value as CFNumberRef).map(|n| trim_number(n))
            } else if CFGetTypeID(value) == CFBooleanGetTypeID() {
                Some(CFBooleanGetValue(value as CFBooleanRef).to_string())
            } else {
                None
            };
            CFRelease(value);
            out
        };
        text.filter(|s| !s.is_empty())
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0 as CFTypeRef) }
    }
}

unsafe fn is_string(value: CFTypeRef) -> bool {
    CFGetTypeID(value) == CFStringGetTypeID()
}

unsafe fn read_ax_point(value: CFTypeRef) -> Option<(f64, f64)> {
    if CFGetTypeID(value) != AXValueGetTypeID() {
        return None;
    }
    let mut point = CGPoint::default();
    let ok = AXValueGetValue(
        value as AXValueRef,
        kAXValueTypeCGPoint,
        &mut point as *mut CGPoint as *mut c_void,
    );
    if ok {
        Some((point.x, point.y))
    } else {
        None
    }
}

unsafe fn read_ax_size(value: CFTypeRef) -> Option<(f64, f64)> {
    if CFGetTypeID(value) != AXValueGetTypeID() {
        return None;
    }
    let mut size = CGSize::default();
    let ok = AXValueGetValue(
        value as AXValueRef,
        kAXValueTypeCGSize,
        &mut size as *mut CGSize as *mut c_void,
    );
    if ok {
        Some((size.width, size.height))
    } else {
        None
    }
}

/// 整数不显示小数点，其余保留一位。
fn trim_number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.1}")
    }
}

fn app_root(pid: i32) -> Result<AxElement> {
    unsafe {
        AxElement::new(AXUIElementCreateApplication(pid))
            .ok_or_else(|| BackendError::Failed(format!("cannot attach to pid {pid}")))
    }
}

/// 找出目标窗口在应用根 `AXChildren` 里的下标。
///
/// AX 元素不直接暴露 CGWindowID，因此用屏幕矩形匹配。窗口的位置与尺寸来自
/// `CGWindowList`，两者坐标系一致，匹配就是比较原点。
///
/// 匹配不上时直接报错，不退回应用根 —— 否则调用方会拿到一份范围不符的树。
fn find_window_index(app: &AxElement, attrs: &Attrs, window_id: u64) -> Result<usize> {
    let bounds = crate::MacBackend::new()
        .windows(true)
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.id == window_id)
        .map(|w| w.bounds)
        .ok_or_else(|| {
            BackendError::Failed(format!(
                "窗口 {window_id} 不在了（窗口已关闭，或应用重启过导致 id 变了）：重新列一次窗口取 id"
            ))
        })?;

    let children = app.children(attrs.children);
    for (index, child) in children.iter().enumerate() {
        if let Some(candidate) = child.bounds(attrs) {
            if (candidate.x - bounds.x).abs() < 1.0 && (candidate.y - bounds.y).abs() < 1.0 {
                return Ok(index);
            }
        }
    }

    Err(BackendError::Failed(format!(
        "window {window_id} 未能在该应用的 AX 树里按位置匹配到（共 {} 个顶层元素）",
        children.len()
    )))
}

/// 按索引链从应用根走到目标元素。
///
/// 路径语义始终是「从应用根开始、逐层走 `AXChildren` 的下标」，与 `element_tree`
/// 的输出一致，因此两边可以互相印证。
fn resolve(app: &AxElement, attrs: &Attrs, path: &[usize]) -> Result<AxElement> {
    let mut node = retain(app)?;

    for (depth, index) in path.iter().enumerate() {
        let children = node.children(attrs.children);
        let count = children.len();
        let Some(next) = children.into_iter().nth(*index) else {
            return Err(BackendError::Failed(format!(
                "路径 {} 的第 {depth} 段越界：该层只有 {count} 个元素",
                join_path(path)
            )));
        };
        node = next;
    }

    Ok(node)
}

/// 复制一份引用，让遍历循环可以统一持有「当前节点」。
fn retain(element: &AxElement) -> Result<AxElement> {
    unsafe {
        let retained = CFRetain(element.raw() as CFTypeRef);
        AxElement::adopt(retained)
            .ok_or_else(|| BackendError::Failed("cannot retain element".to_string()))
    }
}

// ---------------------------------------------------------------- 对外能力

pub fn element_tree(pid: i32, limits: &TreeLimits, window: Option<u64>) -> Result<Vec<ElementNode>> {
    let attrs = Attrs::new();
    let app = app_root(pid)?;

    // 路径始终相对应用根。指定窗口时只是把遍历范围收窄到该窗口那一支。
    let mut queue: VecDeque<(AxElement, Vec<usize>, usize)> = VecDeque::new();
    match window {
        Some(id) => {
            let index = find_window_index(&app, &attrs, id)?;
            let child = app
                .children(attrs.children)
                .into_iter()
                .nth(index)
                .ok_or_else(|| BackendError::Failed("窗口元素消失".to_string()))?;
            queue.push_back((child, vec![index], 1));
        }
        None => queue.push_back((retain(&app)?, Vec::new(), 0)),
    }

    let mut out = Vec::new();
    let mut truncated_by: Option<&'static str> = None;

    while let Some((element, path, depth)) = queue.pop_front() {
        if out.len() >= limits.max_nodes {
            truncated_by = Some("nodes");
            break;
        }

        let role = element
            .string_attribute(attrs.role)
            .unwrap_or_else(|| "AXUnknown".to_string());
        let children = element.children(attrs.children);
        let child_count = children.len();

        out.push(ElementNode {
            path: path.clone(),
            depth,
            role,
            subrole: element.string_attribute(attrs.subrole),
            title: element
                .string_attribute(attrs.title)
                .or_else(|| element.string_attribute(attrs.description)),
            value: element.value_text(&attrs),
            identifier: element.string_attribute(attrs.identifier),
            bounds: element.bounds(&attrs),
            enabled: element.bool_attribute(attrs.enabled),
            focused: element.bool_attribute(attrs.focused).filter(|f| *f),
            children: child_count,
        });

        if depth >= limits.max_depth {
            if child_count > 0 && truncated_by.is_none() {
                truncated_by = Some("depth");
            }
            continue;
        }

        for (index, child) in children.into_iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(index);
            queue.push_back((child, child_path, depth + 1));
        }
    }

    if let Some(reason) = truncated_by {
        // 截断是明确的，调用方可据此收紧条件再查，而不是以为树就这么大。
        out.push(ElementNode {
            path: Vec::new(),
            depth: 0,
            role: format!("AXTruncated:{}", match reason {
                "depth" => "maxDepth",
                _ => "maxNodes",
            }),
            subrole: None,
            title: None,
            value: None,
            identifier: None,
            bounds: None,
            enabled: None,
            focused: None,
            children: 0,
        });
    }

    Ok(out)
}

pub fn perform_action(pid: i32, path: &[usize], action: ElementAction) -> Result<String> {
    let attrs = Attrs::new();
    let app = app_root(pid)?;
    let element = resolve(&app, &attrs, path)?;

    let name = unsafe { cfstring(action.ax_name()) };
    let status = unsafe { AXUIElementPerformAction(element.raw(), name) };
    unsafe { CFRelease(name as CFTypeRef) };

    if status != 0 {
        // 动作不被支持是最常见的失败，把该元素实际支持的动作一并给出，避免反复试。
        let supported = action_names_of(&element);
        let hint = if supported.is_empty() {
            "该元素不支持任何动作".to_string()
        } else {
            format!("该元素支持：{}", supported.join(", "))
        };
        return Err(BackendError::Failed(format!(
            "{} 执行失败（AXError {status}）：{hint}",
            action.ax_name()
        )));
    }
    Ok(action.ax_name().to_string())
}

pub fn set_value(pid: i32, path: &[usize], value: &str) -> Result<()> {
    let attrs = Attrs::new();
    let app = app_root(pid)?;
    let element = resolve(&app, &attrs, path)?;

    let text = unsafe { cfstring(value) };
    let status =
        unsafe { AXUIElementSetAttributeValue(element.raw(), attrs.value, text as CFTypeRef) };
    unsafe { CFRelease(text as CFTypeRef) };

    if status != 0 {
        return Err(BackendError::Failed(format!(
            "设置 AXValue 失败（AXError {status}）；该元素可能只读"
        )));
    }
    Ok(())
}

pub fn element_actions(pid: i32, path: &[usize]) -> Result<Vec<String>> {
    let attrs = Attrs::new();
    let app = app_root(pid)?;
    let element = resolve(&app, &attrs, path)?;
    Ok(action_names_of(&element))
}

fn action_names_of(element: &AxElement) -> Vec<String> {
    let mut names: CFArrayRef = std::ptr::null();
    let status = unsafe { AXUIElementCopyActionNames(element.raw(), &mut names) };
    if status != 0 || names.is_null() {
        return Vec::new();
    }

    let mut out = Vec::new();
    unsafe {
        let count = CFArrayGetCount(names);
        for index in 0..count {
            let item = CFArrayGetValueAtIndex(names, index);
            if let Some(name) = string_value(item as CFStringRef) {
                out.push(name);
            }
        }
        CFRelease(names as CFTypeRef);
    }
    out
}

/// 屏幕坐标下的元素，用于「这点下面是什么」。
pub fn element_at_position(pid: i32, x: f64, y: f64) -> Result<Vec<usize>> {
    let attrs = Attrs::new();
    let app = app_root(pid)?;
    let mut hit: AXUIElementRef = std::ptr::null_mut();
    let status =
        unsafe { AXUIElementCopyElementAtPosition(app.raw(), x as f32, y as f32, &mut hit) };
    if status != 0 || hit.is_null() {
        return Err(BackendError::Failed(format!(
            "该点没有可查询的元素（AXError {status}）"
        )));
    }
    let target = AxElement(hit);

    // 从根做一次广度优先，找出目标元素的路径。
    let mut queue: VecDeque<(AxElement, Vec<usize>, usize)> = VecDeque::new();
    queue.push_back((retain(&app)?, Vec::new(), 0));
    let limit = 4000;
    let mut visited = 0;

    while let Some((element, path, depth)) = queue.pop_front() {
        visited += 1;
        if visited > limit || depth > 24 {
            break;
        }
        if unsafe { CFEqual(element.raw() as CFTypeRef, target.raw() as CFTypeRef) } {
            let _ = attrs;
            return Ok(path);
        }
        for (index, child) in element.children(attrs.children).into_iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(index);
            queue.push_back((child, child_path, depth + 1));
        }
    }

    Err(BackendError::Failed(
        "该点下的元素不在可遍历的 AX 树里".to_string(),
    ))
}
