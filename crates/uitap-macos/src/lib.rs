//! macOS 后端。
//!
//! 观测走已验证过的系统路径：窗口列表用 `CGWindowListCopyWindowInfo`，
//! 截图沿用 `/usr/sbin/screencapture`（`-o -l <id>` 截窗口不含阴影、`-R` 截屏幕区域），
//! 与既有 Swift 版逐项对齐，避免引入 ScreenCaptureKit 带来的语义差异。
//! 输入合成走 `CGEvent`，投递到 HID 层，与真实输入设备同一路径。

pub mod apps;
pub mod keys;
pub mod mainthread;
mod sys;

use std::path::Path;
use std::process::Command;

use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace,
};

use uitap_core::backend::{
    ActivateTarget, Backend, BackendError, CaptureMode, CaptureTarget, DisplayInfo, Modifier,
    MouseButton, Permissions, RawCapture, Result, RunningApp, WindowInfo,
};
use uitap_core::geom::{Point, Rect};
use uitap_core::pixels;
use uitap_core::store;

use sys::*;

const SCREENCAPTURE: &str = "/usr/sbin/screencapture";

/// 拖动分段的每步最小间隔，避免事件被合帧丢弃。
const DRAG_STEP_MIN_MS: u64 = 8;

#[derive(Debug, Default)]
pub struct MacBackend;

impl MacBackend {
    pub fn new() -> Self {
        MacBackend
    }

    fn window_by_id(&self, id: u64) -> Result<WindowInfo> {
        self.windows(true)?
            .into_iter()
            .find(|w| w.id == id)
            .ok_or_else(|| BackendError::Failed(format!("window {id} not found")))
    }
}

/// 通过辅助功能接口把目标进程置前。
///
/// 这是同步请求，不依赖本进程的 run loop 或退出时机；用的是我们本来就要求的
/// 辅助功能权限，因此不引入新的系统授权。失败时静默返回，由调用方兜底。
fn raise_via_accessibility(pid: i32) -> bool {
    unsafe {
        let element = AXUIElementCreateApplication(pid);
        if element.is_null() {
            return false;
        }
        // kAXFrontmostAttribute 在头文件里是 #define CFSTR("AXFrontmost")，没有可链接符号，
        // 因此自己构造这个字符串。
        let attribute = cfstring("AXFrontmost");
        let status =
            AXUIElementSetAttributeValue(element, attribute, kCFBooleanTrue as CFTypeRef);
        CFRelease(attribute as CFTypeRef);
        CFRelease(element as CFTypeRef);
        status == 0
    }
}

/// 让出 run loop 一小段时间。AppKit 的调用需要本进程处理事件才会落地。
pub(crate) fn spin_run_loop(seconds: f64) {
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false);
    }
}

fn sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

impl Backend for MacBackend {
    fn permissions(&self) -> Permissions {
        unsafe {
            Permissions {
                screen_recording: CGPreflightScreenCaptureAccess(),
                accessibility: AXIsProcessTrusted(),
            }
        }
    }

    fn displays(&self) -> Result<Vec<DisplayInfo>> {
        unsafe {
            let mut count: u32 = 0;
            if CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) != 0 || count == 0 {
                return Err(BackendError::Failed("no active display".into()));
            }
            let mut ids = vec![0u32; count as usize];
            if CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) != 0 {
                return Err(BackendError::Failed("cannot enumerate displays".into()));
            }
            ids.truncate(count as usize);

            let main_id = CGMainDisplayID();
            Ok(ids
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    let bounds = CGDisplayBounds(id);
                    DisplayInfo {
                        index,
                        id: id as u64,
                        bounds: Rect::new(
                            bounds.origin.x,
                            bounds.origin.y,
                            bounds.size.width,
                            bounds.size.height,
                        ),
                        pixel_width: CGDisplayPixelsWide(id),
                        pixel_height: CGDisplayPixelsHigh(id),
                        main: id == main_id,
                    }
                })
                .collect())
        }
    }

    fn windows(&self, include_offscreen: bool) -> Result<Vec<WindowInfo>> {
        let option = if include_offscreen {
            kCGWindowListOptionAll
        } else {
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements
        };

        unsafe {
            let array = CGWindowListCopyWindowInfo(option, kCGNullWindowID);
            if array.is_null() {
                return Err(BackendError::Failed(
                    "CGWindowListCopyWindowInfo returned nothing".into(),
                ));
            }

            let count = CFArrayGetCount(array);
            let mut out = Vec::new();

            // 键名只构造一次，循环内复用。
            let k_number = cfstring("kCGWindowNumber");
            let k_pid = cfstring("kCGWindowOwnerPID");
            let k_owner = cfstring("kCGWindowOwnerName");
            let k_name = cfstring("kCGWindowName");
            let k_bounds = cfstring("kCGWindowBounds");
            let k_layer = cfstring("kCGWindowLayer");
            let k_onscreen = cfstring("kCGWindowIsOnscreen");

            for index in 0..count {
                let dict = CFArrayGetValueAtIndex(array, index) as CFDictionaryRef;
                if dict.is_null() {
                    continue;
                }

                let Some(frame) = rect_from_dict(dict_value(dict, k_bounds) as CFDictionaryRef)
                else {
                    continue;
                };
                if frame.size.width < 2.0 || frame.size.height < 2.0 {
                    continue;
                }

                let Some(id) = number_i64(dict_value(dict, k_number) as CFNumberRef) else {
                    continue;
                };
                let Some(pid) = number_i64(dict_value(dict, k_pid) as CFNumberRef) else {
                    continue;
                };

                out.push(WindowInfo {
                    id: id as u64,
                    pid: pid as i32,
                    app: string_value(dict_value(dict, k_owner) as CFStringRef).unwrap_or_default(),
                    title: string_value(dict_value(dict, k_name) as CFStringRef).unwrap_or_default(),
                    bounds: Rect::new(
                        frame.origin.x,
                        frame.origin.y,
                        frame.size.width,
                        frame.size.height,
                    ),
                    layer: number_i64(dict_value(dict, k_layer) as CFNumberRef).unwrap_or(0) as i32,
                    onscreen: number_i64(dict_value(dict, k_onscreen) as CFNumberRef).unwrap_or(1) != 0,
                    z: index as usize,
                });
            }

            for key in [k_number, k_pid, k_owner, k_name, k_bounds, k_layer, k_onscreen] {
                CFRelease(key as CFTypeRef);
            }
            CFRelease(array);

            Ok(out)
        }
    }

    fn capture(&self, target: &CaptureTarget, out: &Path) -> Result<RawCapture> {
        store::prune(240);

        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return Err(BackendError::Failed(format!(
                    "output directory does not exist: {}",
                    parent.display()
                )));
            }
        }

        let mut args: Vec<String> = vec!["-x".into(), "-t".into(), "png".into()];
        let (mode, window_id, origin, point_width, point_height);

        match target {
            CaptureTarget::Window(id) => {
                let win = self.window_by_id(*id)?;
                args.push("-o".into());
                args.push("-l".into());
                args.push(id.to_string());
                mode = CaptureMode::Window;
                window_id = Some(*id);
                origin = Point::new(win.bounds.x, win.bounds.y);
                point_width = win.bounds.w;
                point_height = win.bounds.h;
            }
            CaptureTarget::Region(region) => {
                let r = region.integral();
                push_region(&mut args, &r);
                mode = CaptureMode::Region;
                window_id = None;
                origin = Point::new(r.x, r.y);
                point_width = r.w;
                point_height = r.h;
            }
            CaptureTarget::Screen { display_index } => {
                let displays = self.displays()?;
                let index =
                    display_index.unwrap_or_else(|| displays.iter().position(|d| d.main).unwrap_or(0));
                let display = displays.get(index).ok_or_else(|| {
                    BackendError::Failed(format!("display index {index} out of range"))
                })?;
                let r = display.bounds.integral();
                push_region(&mut args, &r);
                mode = CaptureMode::Screen;
                window_id = None;
                origin = Point::new(r.x, r.y);
                point_width = r.w;
                point_height = r.h;
            }
        }

        args.push(out.to_string_lossy().into_owned());

        let output = Command::new(SCREENCAPTURE)
            .args(&args)
            .output()
            .map_err(|e| BackendError::Failed(format!("cannot run {SCREENCAPTURE}: {e}")))?;

        if !output.status.success() {
            return Err(BackendError::Failed(format!(
                "screencapture failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        // screencapture 拒绝写入时退出码仍可能为 0，必须核实产物。
        if !out.exists() {
            return Err(BackendError::Failed(format!(
                "screencapture wrote nothing to {}: {}",
                out.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let (pixel_width, pixel_height) = pixels::pixel_size(out)
            .map_err(|e| BackendError::Failed(format!("cannot read capture {}: {e}", out.display())))?;

        Ok(RawCapture {
            path: out.to_path_buf(),
            pixel_width,
            pixel_height,
            origin,
            point_width,
            point_height,
            mode,
            window_id,
        })
    }

    fn move_to(&self, at: Point) -> Result<()> {
        unsafe {
            let source = event_source()?;
            post_mouse(source, kCGEventMouseMoved, at, kCGMouseButtonLeft, 1);
            CFRelease(source as CFTypeRef);
        }
        Ok(())
    }

    fn click(&self, at: Point, button: MouseButton, count: u32) -> Result<()> {
        unsafe {
            let source = event_source()?;
            post_mouse(source, kCGEventMouseMoved, at, button_code(button), 1);
            sleep_ms(40);

            let (down, up) = button_events(button);
            let total = count.max(1);
            for index in 1..=total {
                post_mouse(source, down, at, button_code(button), index as i64);
                sleep_ms(40);
                post_mouse(source, up, at, button_code(button), index as i64);
                if index < total {
                    sleep_ms(70);
                }
            }
            CFRelease(source as CFTypeRef);
        }
        Ok(())
    }

    fn drag(&self, from: Point, to: Point, button: MouseButton, duration_ms: u64) -> Result<()> {
        unsafe {
            let source = event_source()?;
            post_mouse(source, kCGEventMouseMoved, from, button_code(button), 1);
            sleep_ms(60);

            let (down, _) = button_events(button);
            let dragged = drag_event(button);
            post_mouse(source, down, from, button_code(button), 1);

            let segments = 20u64;
            let per_step = (duration_ms / segments).max(DRAG_STEP_MIN_MS);
            for index in 1..=segments {
                let t = index as f64 / segments as f64;
                let point = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                post_mouse(source, dragged, point, button_code(button), 1);
                sleep_ms(per_step);
            }

            sleep_ms(40);
            post_mouse(source, up_event(button), to, button_code(button), 1);
            CFRelease(source as CFTypeRef);
        }
        Ok(())
    }

    fn scroll(&self, at: Option<Point>, dx: i32, dy: i32) -> Result<()> {
        unsafe {
            let source = event_source()?;
            if let Some(point) = at {
                post_mouse(source, kCGEventMouseMoved, point, kCGMouseButtonLeft, 1);
                sleep_ms(40);
            }
            let event =
                CGEventCreateScrollWheelEvent(source, kCGScrollEventUnitPixel, 2, dy, dx);
            if !event.is_null() {
                CGEventPost(kCGHIDEventTap, event);
                CFRelease(event as CFTypeRef);
            }
            CFRelease(source as CFTypeRef);
        }
        Ok(())
    }

    fn type_text(&self, text: &str, delay_ms: u64) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        unsafe {
            let source = event_source()?;
            let units: Vec<u16> = text.encode_utf16().collect();

            let send = |slice: &[u16]| {
                if slice.is_empty() {
                    return;
                }
                for is_down in [true, false] {
                    let event = CGEventCreateKeyboardEvent(source, 0, is_down);
                    if !event.is_null() {
                        CGEventKeyboardSetUnicodeString(event, slice.len(), slice.as_ptr());
                        CGEventPost(kCGHIDEventTap, event);
                        CFRelease(event as CFTypeRef);
                    }
                }
            };

            if delay_ms == 0 {
                // 单次事件过长会被系统丢弃，按 20 个 UTF-16 单元分段。
                for chunk in units.chunks(20) {
                    send(chunk);
                    sleep_ms(2);
                }
            } else {
                let mut index = 0usize;
                while index < units.len() {
                    let unit = units[index];
                    // 代理对需要两个单元一起投递。
                    let take = if (0xD800..=0xDBFF).contains(&unit) && index + 1 < units.len() {
                        2
                    } else {
                        1
                    };
                    send(&units[index..index + take]);
                    index += take;
                    sleep_ms(delay_ms);
                }
            }
            CFRelease(source as CFTypeRef);
        }
        Ok(())
    }

    fn key(&self, key_code: u16, modifiers: &[Modifier]) -> Result<()> {
        let mut flags: u64 = 0;
        for modifier in modifiers {
            flags |= match modifier {
                Modifier::Cmd => kCGEventFlagMaskCommand,
                Modifier::Shift => kCGEventFlagMaskShift,
                Modifier::Opt => kCGEventFlagMaskAlternate,
                Modifier::Ctrl => kCGEventFlagMaskControl,
                Modifier::Fn => kCGEventFlagMaskSecondaryFn,
            };
        }

        unsafe {
            let source = event_source()?;
            for is_down in [true, false] {
                let event = CGEventCreateKeyboardEvent(source, key_code, is_down);
                if !event.is_null() {
                    CGEventSetFlags(event, flags);
                    CGEventPost(kCGHIDEventTap, event);
                    CFRelease(event as CFTypeRef);
                }
            }
            CFRelease(source as CFTypeRef);
        }
        sleep_ms(20);
        Ok(())
    }

    fn activate(&self, target: &ActivateTarget) -> Result<RunningApp> {
        // AppKit 的激活只有在本进程主线程执行才会落地：工作线程发起时会被系统搁置，
        // 因此这里把整个激活流程（请求 + 确认）投递到主线程。
        let owned = target.clone();
        if let Some(result) = mainthread::run_on_main(move || MacBackend::activate_inline(&owned)) {
            return result;
        }
        MacBackend::activate_inline(target)
    }

    fn frontmost(&self) -> Result<RunningApp> {
        let workspace = NSWorkspace::sharedWorkspace();
        match workspace.frontmostApplication() {
            Some(app) => Ok(RunningApp {
                frontmost: true,
                ..app_info(&app)
            }),
            None => Err(BackendError::Failed("no frontmost application".into())),
        }
    }
}

impl MacBackend {
    /// 激活的实际实现。调用方负责保证它在主线程执行。
    fn activate_inline(target: &ActivateTarget) -> Result<RunningApp> {
        let backend = MacBackend::new();
        let workspace = NSWorkspace::sharedWorkspace();

        let app = match target {
            ActivateTarget::Pid(pid) => {
                NSRunningApplication::runningApplicationWithProcessIdentifier(*pid)
            }
            ActivateTarget::Window(id) => {
                let win = backend.window_by_id(*id)?;
                NSRunningApplication::runningApplicationWithProcessIdentifier(win.pid)
            }
            ActivateTarget::App(name) => {
                let needle = name.to_ascii_lowercase();
                let running = workspace.runningApplications();
                let mut exact = None;
                let mut partial = None;
                for app in running.iter() {
                    let localized = app
                        .localizedName()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let bundle = app
                        .bundleIdentifier()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let prohibited = app.activationPolicy()
                        == NSApplicationActivationPolicy::Prohibited;

                    match apps::classify(&needle, &localized, &bundle, prohibited) {
                        Some(apps::MatchKind::Exact) => {
                            exact = Some(app.clone());
                            break;
                        }
                        Some(apps::MatchKind::Partial) => {
                            if partial.is_none() {
                                partial = Some(app.clone());
                            }
                        }
                        None => {}
                    }
                }
                exact.or(partial)
            }
        };

        let app = app.ok_or_else(|| BackendError::Failed("application not found".into()))?;
        let target_pid = app.processIdentifier();
        let info = app_info(&app);

        // macOS 14 起 ActivateIgnoringOtherApps 已无效果，且 activate 的请求可能被延迟到
        // 本进程让出控制权之后。因此先直接向目标进程的辅助功能接口要求置前，再轮询确认。
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2500);
        let mut last_request = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let frontmost = loop {
            let now_frontmost = backend
                .frontmost()
                .map(|f| f.pid == target_pid)
                .unwrap_or(false);
            if now_frontmost || std::time::Instant::now() >= deadline {
                break now_frontmost;
            }
            if last_request.elapsed() >= std::time::Duration::from_millis(400) {
                // AX 置前是同步请求，用它作为主路径；activate 作为兜底。
                raise_via_accessibility(target_pid);
                let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
                last_request = std::time::Instant::now();
            }
            // 让出 run loop，让系统把激活派发进来；死等 sleep 会让请求一直排队。
            spin_run_loop(0.04);
        };

        Ok(RunningApp { frontmost, ..info })
    }
}

fn push_region(args: &mut Vec<String>, r: &Rect) {
    args.push("-R".into());
    args.push(format!(
        "{},{},{},{}",
        r.x.round() as i64,
        r.y.round() as i64,
        r.w.round() as i64,
        r.h.round() as i64
    ));
}

fn button_code(button: MouseButton) -> CGMouseButton {
    match button {
        MouseButton::Left => kCGMouseButtonLeft,
        MouseButton::Right => kCGMouseButtonRight,
        MouseButton::Middle => kCGMouseButtonCenter,
    }
}

fn button_events(button: MouseButton) -> (CGEventType, CGEventType) {
    match button {
        MouseButton::Left => (kCGEventLeftMouseDown, kCGEventLeftMouseUp),
        MouseButton::Right => (kCGEventRightMouseDown, kCGEventRightMouseUp),
        MouseButton::Middle => (kCGEventOtherMouseDown, kCGEventOtherMouseUp),
    }
}

fn up_event(button: MouseButton) -> CGEventType {
    button_events(button).1
}

fn drag_event(button: MouseButton) -> CGEventType {
    match button {
        MouseButton::Left => kCGEventLeftMouseDragged,
        MouseButton::Right => kCGEventRightMouseDragged,
        MouseButton::Middle => kCGEventOtherMouseDragged,
    }
}

unsafe fn event_source() -> Result<CGEventSourceRef> {
    let source = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
    if source.is_null() {
        return Err(BackendError::Failed("CGEventSourceCreate failed".into()));
    }
    Ok(source)
}

unsafe fn post_mouse(
    source: CGEventSourceRef,
    kind: CGEventType,
    point: Point,
    button: CGMouseButton,
    click_state: i64,
) {
    let event = CGEventCreateMouseEvent(
        source,
        kind,
        CGPoint {
            x: point.x,
            y: point.y,
        },
        button,
    );
    if event.is_null() {
        return;
    }
    if click_state > 1 {
        CGEventSetIntegerValueField(event, kCGMouseEventClickState, click_state);
    }
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event as CFTypeRef);
}

fn app_info(app: &NSRunningApplication) -> RunningApp {
    RunningApp {
        app: app.localizedName()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        pid: app.processIdentifier(),
        bundle_id: app.bundleIdentifier()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        frontmost: false,
    }
}

/// `/usr/sbin/screencapture` 是否存在，供 `doctor` 报告依赖状态。
pub fn screencapture_available() -> bool {
    Path::new(SCREENCAPTURE).exists()
}

pub const SCREENCAPTURE_PATH: &str = SCREENCAPTURE;
