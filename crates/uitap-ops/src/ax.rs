//! 辅助功能元素的操作：取树、按条件查、执行动作、设值、等元素出现。

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use uitap_core::backend::{
    Backend, ElementAction, ElementNode, TreeLimits, join_path, parse_path,
};
use uitap_core::jsonout::rect_json;

use crate::types::OpResult;

/// 元素操作的目标。三者给出其一即可，优先取更具体的。
#[derive(Clone, Debug)]
pub enum AppTarget {
    App(String),
    Pid(i32),
    Window(u64),
}

impl AppTarget {
    /// 从「应用名 / pid / 窗口 id」中挑出给出的那一个。
    pub fn from_parts(app: Option<String>, pid: Option<i32>, window: Option<u64>) -> Option<Self> {
        if let Some(pid) = pid {
            return Some(AppTarget::Pid(pid));
        }
        if let Some(window) = window {
            return Some(AppTarget::Window(window));
        }
        app.map(AppTarget::App)
    }

    pub fn describe(&self) -> String {
        match self {
            AppTarget::App(name) => format!("app={name}"),
            AppTarget::Pid(pid) => format!("pid={pid}"),
            AppTarget::Window(id) => format!("window={id}"),
        }
    }
}

/// 解析目标进程。窗口 id 通过窗口列表换算成所属进程。
pub fn resolve_pid(backend: &dyn Backend, target: &AppTarget) -> OpResult<i32> {
    match target {
        AppTarget::Pid(pid) => Ok(*pid),
        AppTarget::Window(id) => backend
            .windows(true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|w| w.id == *id)
            .map(|w| w.pid)
            .ok_or_else(|| {
                format!("窗口 {id} 不在了（窗口已关闭，或应用重启过导致 id 变了）：重新列一次窗口取 id")
            }),
        AppTarget::App(name) => backend
            .running_app(name)
            .map(|app| app.pid)
            .map_err(|e| format!("{}（{e}）", target.describe())),
    }
}

/// 查元素的条件。给出的每一项都必须满足；字符串按「包含」匹配，大小写不敏感。
#[derive(Clone, Debug, Default)]
pub struct ElementQuery {
    pub role: Option<String>,
    /// 次级角色，如 AXCloseButton、AXStandardWindow。区分同名控件的关键。
    pub subrole: Option<String>,
    pub title: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
    /// 只看 enabled 为真的元素。
    pub enabled_only: bool,
}

impl ElementQuery {
    pub fn is_empty(&self) -> bool {
        self.role.is_none()
            && self.subrole.is_none()
            && self.title.is_none()
            && self.value.is_none()
            && self.identifier.is_none()
            && !self.enabled_only
    }
}

fn contains(haystack: Option<&str>, needle: &str) -> bool {
    match haystack {
        Some(text) => text.to_lowercase().contains(&needle.to_lowercase()),
        None => false,
    }
}

/// 判断元素是否命中查询。纯函数，便于单测覆盖匹配语义。
pub fn matches(node: &ElementNode, query: &ElementQuery) -> bool {
    if let Some(role) = &query.role {
        if !contains(Some(&node.role), role) {
            return false;
        }
    }
    if let Some(subrole) = &query.subrole {
        if !contains(node.subrole.as_deref(), subrole) {
            return false;
        }
    }
    if let Some(title) = &query.title {
        if !contains(node.title.as_deref(), title) {
            return false;
        }
    }
    if let Some(value) = &query.value {
        if !contains(node.value.as_deref(), value) {
            return false;
        }
    }
    if let Some(identifier) = &query.identifier {
        if !contains(node.identifier.as_deref(), identifier) {
            return false;
        }
    }
    if query.enabled_only && node.enabled != Some(true) {
        return false;
    }
    true
}

/// 元素的紧凑 JSON。空字段不写，避免输出里全是 `null`。
pub fn element_json(node: &ElementNode) -> Value {
    let mut map = Map::new();
    map.insert("path".into(), Value::String(node.path_text()));
    map.insert("depth".into(), Value::from(node.depth));
    map.insert("role".into(), Value::String(node.role.clone()));
    if let Some(subrole) = &node.subrole {
        map.insert("subrole".into(), Value::String(subrole.clone()));
    }
    if let Some(title) = &node.title {
        map.insert("title".into(), Value::String(title.clone()));
    }
    if let Some(value) = &node.value {
        map.insert("value".into(), Value::String(value.clone()));
    }
    if let Some(identifier) = &node.identifier {
        map.insert("identifier".into(), Value::String(identifier.clone()));
    }
    if let Some(bounds) = node.bounds {
        map.insert("bounds".into(), rect_json(bounds));
    }
    if let Some(enabled) = node.enabled {
        map.insert("enabled".into(), Value::Bool(enabled));
    }
    if node.focused == Some(true) {
        map.insert("focused".into(), Value::Bool(true));
    }
    if node.children > 0 {
        map.insert("children".into(), Value::from(node.children));
    }
    Value::Object(map)
}

/// 节点是否是被截断时补的哨兵。
fn is_truncation_marker(node: &ElementNode) -> bool {
    node.role.starts_with("AXTruncated")
}

pub fn tree(
    backend: &dyn Backend,
    pid: i32,
    limits: &TreeLimits,
    window: Option<u64>,
) -> OpResult<Value> {
    let nodes = backend
        .element_tree(pid, limits, window)
        .map_err(|e| e.to_string())?;

    let truncated = nodes.iter().any(is_truncation_marker);
    let visible: Vec<&ElementNode> = nodes.iter().filter(|n| !is_truncation_marker(n)).collect();

    let mut map = Map::new();
    map.insert("pid".into(), Value::from(pid));
    if let Some(id) = window {
        map.insert("window".into(), Value::from(id));
    }
    map.insert("count".into(), Value::from(visible.len()));
    if truncated {
        // 说明输出被边界截断，调用方应收紧条件再查，而不是以为树就这么大。
        map.insert("truncated".into(), Value::Bool(true));
    }
    map.insert(
        "nodes".into(),
        Value::Array(visible.iter().map(|n| element_json(n)).collect()),
    );
    Ok(Value::Object(map))
}

pub fn find(
    backend: &dyn Backend,
    pid: i32,
    query: &ElementQuery,
    limits: &TreeLimits,
    window: Option<u64>,
    limit: usize,
) -> OpResult<Value> {
    let nodes = backend
        .element_tree(pid, limits, window)
        .map_err(|e| e.to_string())?;

    let mut hits: Vec<&ElementNode> = nodes
        .iter()
        .filter(|n| !is_truncation_marker(n) && matches(n, query))
        .collect();
    let total = hits.len();
    hits.truncate(limit);

    let mut map = Map::new();
    map.insert("pid".into(), Value::from(pid));
    map.insert("count".into(), Value::from(total));
    if total > hits.len() {
        map.insert("limit".into(), Value::from(limit));
    }
    map.insert(
        "elements".into(),
        Value::Array(hits.iter().map(|n| element_json(n)).collect()),
    );
    Ok(Value::Object(map))
}

pub fn act(backend: &dyn Backend, pid: i32, path_text: &str, action: ElementAction) -> OpResult<Value> {
    let path = parse_path(path_text).map_err(|e| e.to_string())?;
    let delivered = backend
        .element_act(pid, &path, action)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("pid".into(), Value::from(pid));
    map.insert("path".into(), Value::String(join_path(&path)));
    map.insert("action".into(), Value::String(action.as_str().into()));
    map.insert("delivered".into(), Value::String(delivered));
    Ok(Value::Object(map))
}

pub fn set_value(backend: &dyn Backend, pid: i32, path_text: &str, value: &str) -> OpResult<Value> {
    let path = parse_path(path_text).map_err(|e| e.to_string())?;
    backend
        .element_set_value(pid, &path, value)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("pid".into(), Value::from(pid));
    map.insert("path".into(), Value::String(join_path(&path)));
    map.insert("length".into(), Value::from(value.chars().count()));
    Ok(Value::Object(map))
}

pub fn actions(backend: &dyn Backend, pid: i32, path_text: &str) -> OpResult<Value> {
    let path = parse_path(path_text).map_err(|e| e.to_string())?;
    let names = backend
        .element_actions(pid, &path)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("pid".into(), Value::from(pid));
    map.insert("path".into(), Value::String(join_path(&path)));
    map.insert("count".into(), Value::from(names.len()));
    map.insert(
        "actions".into(),
        Value::Array(names.into_iter().map(Value::String).collect()),
    );
    Ok(Value::Object(map))
}

/// 轮询直到出现符合条件的元素，或超时。找到时返回首个命中项。
pub fn wait_for(
    backend: &dyn Backend,
    pid: i32,
    query: &ElementQuery,
    limits: &TreeLimits,
    window: Option<u64>,
    timeout_ms: u64,
    interval_ms: u64,
) -> OpResult<Value> {
    if query.is_empty() {
        return Err("至少给一个查询条件，否则第一次就会命中任意元素".to_string());
    }

    let interval = interval_ms.max(50);
    let started = Instant::now();
    let mut attempts = 0usize;

    let first = backend
        .element_tree(pid, limits, window)
        .map_err(|e| e.to_string())?;
    let mut matched = first
        .iter()
        .find(|n| !is_truncation_marker(n) && matches(n, query))
        .cloned();
    attempts += 1;

    while matched.is_none() && started.elapsed().as_millis() < timeout_ms as u128 {
        std::thread::sleep(Duration::from_millis(interval));
        let nodes = backend
            .element_tree(pid, limits, window)
            .map_err(|e| e.to_string())?;
        attempts += 1;
        matched = nodes
            .iter()
            .find(|n| !is_truncation_marker(n) && matches(n, query))
            .cloned();
    }

    let mut map = Map::new();
    map.insert("pid".into(), Value::from(pid));
    map.insert("found".into(), Value::Bool(matched.is_some()));
    map.insert("attempts".into(), Value::from(attempts));
    map.insert(
        "elapsedMs".into(),
        Value::from(started.elapsed().as_millis() as u64),
    );
    if let Some(node) = matched {
        map.insert("element".into(), element_json(&node));
    }
    Ok(Value::Object(map))
}

/// 元素中心点的屏幕坐标，供需要点击时与坐标类工具衔接。
pub fn element_center(node: &ElementNode) -> Option<uitap_core::geom::Point> {
    let bounds = node.bounds?;
    Some(uitap_core::geom::Point::new(
        bounds.x + bounds.w / 2.0,
        bounds.y + bounds.h / 2.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uitap_core::backend::ElementNode;
    use uitap_core::geom::Rect;

    fn node(role: &str, title: Option<&str>, value: Option<&str>) -> ElementNode {
        ElementNode {
            path: vec![0, 1],
            depth: 2,
            role: role.to_string(),
            subrole: None,
            title: title.map(str::to_string),
            value: value.map(str::to_string),
            identifier: None,
            bounds: Some(Rect::new(10.0, 20.0, 100.0, 30.0)),
            enabled: Some(true),
            focused: None,
            children: 0,
        }
    }

    #[test]
    fn query_matches_case_insensitively_on_substring() {
        let button = node("AXButton", Some("提交订单"), None);
        assert!(matches(&button, &ElementQuery {
            title: Some("提交".into()),
            ..Default::default()
        }));
        assert!(matches(&button, &ElementQuery {
            role: Some("axbutton".into()),
            ..Default::default()
        }));
        assert!(!matches(&button, &ElementQuery {
            title: Some("取消".into()),
            ..Default::default()
        }));
    }

    #[test]
    fn all_given_conditions_must_hold() {
        let button = node("AXButton", Some("提交"), None);
        // 条件都满足
        assert!(matches(&button, &ElementQuery {
            role: Some("Button".into()),
            title: Some("提交".into()),
            ..Default::default()
        }));
        // 只要有一项不满足就不命中
        assert!(!matches(&button, &ElementQuery {
            role: Some("Button".into()),
            title: Some("取消".into()),
            ..Default::default()
        }));
    }

    #[test]
    fn missing_attribute_never_matches_that_condition() {
        let silent = node("AXGroup", None, None);
        assert!(!matches(&silent, &ElementQuery {
            title: Some("".into()),
            ..Default::default()
        }));
        // 没有条件的查询命中一切（wait_for 会拒绝空条件）
        assert!(matches(&silent, &ElementQuery::default()));
    }

    #[test]
    fn enabled_only_filters_out_disabled_and_unknown() {
        let mut disabled = node("AXButton", Some("提交"), None);
        disabled.enabled = Some(false);
        assert!(!matches(&disabled, &ElementQuery {
            enabled_only: true,
            ..Default::default()
        }));

        let mut unknown = node("AXButton", Some("提交"), None);
        unknown.enabled = None;
        assert!(!matches(&unknown, &ElementQuery {
            enabled_only: true,
            ..Default::default()
        }));

        assert!(matches(&node("AXButton", Some("提交"), None), &ElementQuery {
            enabled_only: true,
            ..Default::default()
        }));
    }

    #[test]
    fn subrole_distinguishes_same_role_controls() {
        let mut close = node("AXButton", None, None);
        close.subrole = Some("AXCloseButton".into());
        let mut fullscreen = node("AXButton", None, None);
        fullscreen.subrole = Some("AXFullScreenButton".into());

        let query = ElementQuery {
            role: Some("AXButton".into()),
            subrole: Some("CloseButton".into()),
            ..Default::default()
        };
        assert!(matches(&close, &query));
        assert!(!matches(&fullscreen, &query), "不应把全屏按钮当成关闭按钮");

        // subrole 缺失的元素不满足 subrole 条件
        assert!(!matches(&node("AXButton", None, None), &query));
    }

    #[test]
    fn empty_query_is_detected() {
        assert!(ElementQuery::default().is_empty());
        assert!(!ElementQuery {
            title: Some("x".into()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ElementQuery {
            enabled_only: true,
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn element_json_omits_empty_fields() {
        let mut bare = node("AXGroup", None, None);
        bare.bounds = None;
        bare.enabled = None;
        bare.children = 0;
        let json = element_json(&bare);
        let text = serde_json::to_string(&json).unwrap();
        assert!(text.contains("\"role\":\"AXGroup\""));
        assert!(!text.contains("title"));
        assert!(!text.contains("bounds"));
        assert!(!text.contains("enabled"));
        assert!(!text.contains("children"));
        // path 与 depth 始终输出
        assert!(text.contains("\"path\":\"0.1\""));
        assert!(text.contains("\"depth\":2"));
    }

    #[test]
    fn truncation_marker_is_recognized() {
        let mut marker = node("AXTruncated:maxNodes", None, None);
        marker.path = Vec::new();
        assert!(is_truncation_marker(&marker));
        assert!(!is_truncation_marker(&node("AXButton", None, None)));
    }

    #[test]
    fn app_target_prefers_more_specific_identifiers() {
        let picked = AppTarget::from_parts(Some("Safari".into()), Some(42), Some(7)).unwrap();
        assert!(matches!(picked, AppTarget::Pid(42)));

        let picked = AppTarget::from_parts(Some("Safari".into()), None, Some(7)).unwrap();
        assert!(matches!(picked, AppTarget::Window(7)));

        let picked = AppTarget::from_parts(Some("Safari".into()), None, None).unwrap();
        assert!(matches!(picked, AppTarget::App(_)));

        assert!(AppTarget::from_parts(None, None, None).is_none());
    }

    #[test]
    fn element_center_uses_bounds_middle() {
        let center = element_center(&node("AXButton", None, None)).unwrap();
        assert_eq!(center.x, 60.0);
        assert_eq!(center.y, 35.0);
    }
}
