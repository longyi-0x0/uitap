//! 工具声明。名称、描述与 JSON Schema 与既有 MCP 契约保持一致。

use std::sync::Arc;

use rmcp::model::{JsonObject, Tool};
use serde_json::{json, Value};

fn schema(value: Value) -> Arc<JsonObject> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(JsonObject::new()),
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), Value::String("object".into()));
    map.insert("properties".into(), properties);
    if !required.is_empty() {
        map.insert(
            "required".into(),
            Value::Array(required.iter().map(|k| Value::String((*k).into())).collect()),
        );
    }
    Value::Object(map)
}

fn point_array(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "number" }, "description": description })
}

/// 观测目标三选一：窗口 id、屏幕区域、显示器序号。
fn target_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "window",
            json!({ "type": "number", "description": "窗口 id（ui_windows 的 id 字段）" }),
        ),
        (
            "region",
            point_array("[x, y, w, h] 全局点坐标"),
        ),
        (
            "display",
            json!({ "type": "number", "description": "显示器序号（ui_screens 的 index 字段）" }),
        ),
    ]
}

/// 输入类工具共用的互斥参数说明。抽成常量是为了七个工具的文案不会各说各话。
const WAIT_MS_DESC: &str = "被别的 agent 占用时最多等多久，默认 5000";
const NO_LOCK_DESC: &str = "true 时跳过互斥检查；仅在确认无并发时用";

/// 输入类工具共用的互斥参数，供 `merge` 形式的 schema 使用。
fn lease_properties() -> Vec<(&'static str, Value)> {
    vec![
        ("waitMs", json!({ "type": "number", "description": WAIT_MS_DESC })),
        ("noLock", json!({ "type": "boolean", "description": NO_LOCK_DESC })),
    ]
}

/// 元素类工具的目标：应用名 / pid / 窗口 id 三选一。
fn app_target_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "app",
            json!({ "type": "string", "description": "应用名或 bundle id，如 Finder" }),
        ),
        ("pid", json!({ "type": "number", "description": "进程 id" })),
        (
            "window",
            json!({ "type": "number", "description": "窗口 id，取其所属进程" }),
        ),
    ]
}

/// 树遍历的边界。
fn tree_properties() -> Vec<(&'static str, Value)> {
    vec![
        (
            "depth",
            json!({ "type": "number", "description": "最大层数，默认 12" }),
        ),
        (
            "maxNodes",
            json!({ "type": "number", "description": "最大节点数，默认 200；超出时返回带 truncated" }),
        ),
    ]
}

/// 元素查询条件，`ui_find` 与 `ui_wait_for` 共用。字符串按「包含」匹配，大小写不敏感。
fn query_properties() -> Vec<(&'static str, Value)> {
    vec![
        ("role", json!({ "type": "string", "description": "辅助功能角色，如 AXButton、AXTextArea" })),
        (
            "subrole",
            json!({ "type": "string", "description": "次级角色，如 AXCloseButton、AXFullScreenButton；区分同名控件用它" }),
        ),
        ("title", json!({ "type": "string", "description": "标题或描述" })),
        ("value", json!({ "type": "string", "description": "元素当前的值" })),
        ("identifier", json!({ "type": "string", "description": "开发者设定的标识符" })),
        (
            "enabled",
            json!({ "type": "boolean", "description": "只看 enabled 为真的元素（并非所有角色都有该属性）" }),
        ),
    ]
}

/// 等待稳定相关参数，`ui_tap` 与 `ui_wait_stable` 共用。
fn wait_properties() -> Vec<(&'static str, Value)> {
    vec![
        ("timeoutMs", json!({ "type": "number", "description": "等待上限，默认 4000" })),
        (
            "threshold",
            json!({ "type": "number", "description": "判定静止的变化比例上限，默认 0.0006" }),
        ),
        (
            "stableSamples",
            json!({ "type": "number", "description": "连续静止帧数，默认 2" }),
        ),
    ]
}

/// 合并两组属性为一个完整的 object schema。
fn merge(base: &[(&str, Value)], extra: &[(&str, Value)]) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in base.iter().chain(extra.iter()) {
        map.insert((*key).to_string(), value.clone());
    }
    object(Value::Object(map), &[])
}

pub fn tool_list() -> Vec<Tool> {
    vec![
        Tool::new(
            "ui_doctor",
            "检查屏幕录制与辅助功能授权状态。授权缺失时先看这里。",
            schema(object(json!({}), &[])),
        ),
        Tool::new(
            "ui_screens",
            "列出显示器：bounds 为点坐标、pixels 为像素、scale 为倍率。",
            schema(object(json!({}), &[])),
        ),
        Tool::new(
            "ui_windows",
            "列出窗口。按面积降序，同尺寸时按前后叠放（z 越小越靠前），默认返回 25 个。用 id 做后续截图与激活。",
            schema(object(
                json!({
                    "app": { "type": "string", "description": "按应用名模糊过滤" },
                    "title": { "type": "string", "description": "按标题模糊过滤" },
                    "all": { "type": "boolean", "description": "含不在当前 Space 的窗口" },
                    "frontOnly": { "type": "boolean", "description": "只看前台应用的窗口" },
                    "minWidth": { "type": "number" },
                    "minHeight": { "type": "number" },
                    "layer": { "type": "number", "description": "只看指定窗口层级（0 为普通窗口）" },
                    "limit": { "type": "number", "description": "返回条数上限，默认 25" },
                }),
                &[],
            )),
        ),
        Tool::new(
            "ui_shot",
            "截图并返回锚点（origin/scale）与短 id。默认不返回图像，需要看图时传 includeImage。",
            schema(merge(
                &target_properties(),
                &[
                    ("includeImage", json!({ "type": "boolean", "description": "true 时附带缩放后的图像副本" })),
                    ("maxPx", json!({ "type": "number", "description": "图像副本最长边，默认 1280；截图本身也按它缩放" })),
                    ("path", json!({ "type": "string", "description": "指定输出路径" })),
                ],
            )),
        ),
        Tool::new(
            "ui_zoom",
            "放大看某张截图的局部。uitap 截的图按全局点坐标裁；用户给的图（图旁没有 .json 锚点）按图像像素裁。同时回一段元数据说明这次用的是哪种坐标。",
            schema(object(
                json!({
                    "shot": { "type": "string", "description": "截图 id 或路径" },
                    "region": point_array("[x, y, w, h] 全局点坐标，省略则整图；图旁没有锚点时按图像像素"),
                    "maxPx": { "type": "number", "description": "最长边，默认 1400" },
                }),
                &["shot"],
            )),
        ),
        Tool::new(
            "ui_pixel",
            "取若干个点的颜色，返回 #RRGGBB。给 expect 可直接得到颜色是否匹配的判定。返回里的 units 说明这次按点坐标还是图像像素解释。",
            schema(object(
                json!({
                    "shot": { "type": "string", "description": "截图 id 或路径" },
                    "points": {
                        "type": "array",
                        "description": "点坐标数组，形如 [[x, y], ...]；uitap 截的图是全局点坐标，用户给的图是图像像素",
                        "items": { "type": "array", "items": { "type": "number" } },
                    },
                    "expect": {
                        "type": "array",
                        "description": "与 points 一一对应的期望颜色，如 [\"#FF0000\"]；给出后每点附带 match 布尔值",
                        "items": { "type": "string" },
                    },
                    "tolerance": { "type": "number", "description": "expect 的单通道容差，默认 12" },
                }),
                &["shot", "points"],
            )),
        ),
        Tool::new(
            "ui_diff",
            "比对两张截图，返回变化区域。验证界面是否响应首选这个。返回里的 units 说明坐标口径。",
            schema(object(
                json!({
                    "before": { "type": "string", "description": "截图 id 或路径" },
                    "after": { "type": "string", "description": "截图 id 或路径" },
                    "region": point_array("只比对 [x, y, w, h]；before 没有锚点时按图像像素"),
                    "threshold": { "type": "number", "description": "单像素视为变化的色差阈值，默认 24" },
                    "maxRegions": { "type": "number", "description": "返回区域数上限，默认 6" },
                    "minPixels": { "type": "number", "description": "区域最小像素数，默认 12" },
                }),
                &["before", "after"],
            )),
        ),
        Tool::new(
            "ui_wait_stable",
            "等画面不再变化（动画、加载结束）。点完东西再观察前先调它。",
            schema(merge(
                &target_properties(),
                &[
                    ("timeoutMs", json!({ "type": "number", "description": "等待上限，默认 4000" })),
                    ("threshold", json!({ "type": "number", "description": "判定静止的变化比例上限，默认 0.0006" })),
                    ("stableSamples", json!({ "type": "number", "description": "连续静止帧数，默认 2" })),
                    ("intervalMs", json!({ "type": "number", "description": "采样间隔，默认 120" })),
                ],
            )),
        ),
        Tool::new(
            "ui_click",
            "在全局点坐标处点击。",
            schema(object(
                json!({
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"] },
                    "count": { "type": "number", "description": "连击次数，2 为双击" },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &["x", "y"],
            )),
        ),
        Tool::new(
            "ui_drag",
            "从一点拖到另一点。",
            schema(object(
                json!({
                    "from": point_array("起点"),
                    "to": point_array("终点"),
                    "durationMs": { "type": "number", "description": "拖动时长，默认 300" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"] },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &["from", "to"],
            )),
        ),
        Tool::new(
            "ui_scroll",
            "滚动滚轮。dy 为正向上滚，dx 为正向右滚。",
            schema(object(
                json!({
                    "at": point_array("先移到该点再滚"),
                    "dy": { "type": "number" },
                    "dx": { "type": "number" },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &[],
            )),
        ),
        Tool::new(
            "ui_type",
            "键入文本（支持中文）。目标应用需已获得焦点。",
            schema(object(
                json!({
                    "text": { "type": "string" },
                    "delayMs": { "type": "number", "description": "逐字间隔，默认 0；应用丢字时调大" },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &["text"],
            )),
        ),
        Tool::new(
            "ui_key",
            "按组合键，如 \"cmd+shift+t\"、\"esc\"、\"return\"。",
            schema(object(
                json!({
                    "combo": { "type": "string" },
                    "repeat": { "type": "number" },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &["combo"],
            )),
        ),
        Tool::new(
            "ui_activate",
            "把应用切到前台。app / pid / window 三选一。",
            schema(object(
                json!({
                    "app": { "type": "string", "description": "应用名或 bundle id" },
                    "pid": { "type": "number" },
                    "window": { "type": "number", "description": "窗口 id，取其所属应用" },
                    "waitMs": { "type": "number", "description": WAIT_MS_DESC },
                    "noLock": { "type": "boolean", "description": NO_LOCK_DESC },
                }),
                &[],
            )),
        ),
        Tool::new(
            "ui_lease",
            "查看输入互斥租约的状态。多个 agent 共用一个桌面时，输入操作会先取租约；被占用时工具会返回谁在占用与预计等待时间。持有者卡死时用 action=\"clear\" 清除。",
            schema(object(
                json!({
                    "action": {
                        "type": "string",
                        "enum": ["status", "clear"],
                        "description": "status 查看，clear 强制清除（仅在确认持有者已卡死时用）",
                    },
                }),
                &[],
            )),
        ),
        Tool::new(
            "ui_tree",
            "读取应用的辅助功能元素树（拍平，广度优先）。path 是从应用根开始的子索引链，可直接用于 ui_press / ui_set_value / ui_actions。比截图更省 token，且不受遮挡影响。",
            schema(merge(
                &{
                    let mut props = app_target_properties();
                    props.extend(tree_properties());
                    props
                },
                &[],
            )),
        ),
        Tool::new(
            "ui_find",
            "在元素树里按条件查元素。条件按「包含」匹配、大小写不敏感，给出的每一项都必须满足。返回 path 与 bounds，可直接接 ui_press 或换算成点击坐标。",
            schema(merge(
                &{
                    let mut props = app_target_properties();
                    props.extend(query_properties());
                    props.extend(tree_properties());
                    props
                },
                &[(
                    "limit",
                    json!({ "type": "number", "description": "返回条数上限，默认 20" }),
                )],
            )),
        ),
        Tool::new(
            "ui_actions",
            "列出元素支持的动作（AXPress、AXIncrement 等）。列表为空表示该元素不可交互。",
            schema(object(
                json!({
                    "app": { "type": "string" },
                    "pid": { "type": "number" },
                    "window": { "type": "number" },
                    "path": { "type": "string", "description": "元素路径，如 0.1.3" },
                }),
                &["path"],
            )),
        ),
        Tool::new(
            "ui_press",
            "对元素执行动作，默认 press（相当于点击）。走辅助功能接口，不移动鼠标、不切换前台应用。失败时会列出该元素实际支持的动作。",
            schema(object(
                json!({
                    "app": { "type": "string" },
                    "pid": { "type": "number" },
                    "window": { "type": "number" },
                    "path": { "type": "string", "description": "元素路径，来自 ui_find / ui_tree" },
                    "action": {
                        "type": "string",
                        "enum": ["press", "showMenu", "increment", "decrement", "confirm", "cancel", "pick"],
                        "description": "默认 press",
                    },
                }),
                &["path"],
            )),
        ),
        Tool::new(
            "ui_set_value",
            "设置元素的值，用于文本框等可直接写入的控件。走辅助功能接口，不触发键盘输入。",
            schema(object(
                json!({
                    "app": { "type": "string" },
                    "pid": { "type": "number" },
                    "window": { "type": "number" },
                    "path": { "type": "string", "description": "元素路径" },
                    "value": { "type": "string", "description": "要写入的文本" },
                }),
                &["path", "value"],
            )),
        ),
        Tool::new(
            "ui_wait_for",
            "等符合条件的元素出现。用于等待弹窗、加载完成、某个按钮变为可点。比反复截图比对省 token。",
            schema(merge(
                &{
                    let mut props = app_target_properties();
                    props.extend(query_properties());
                    props.extend(tree_properties());
                    props
                },
                &[
                    ("timeoutMs", json!({ "type": "number", "description": "等待上限，默认 5000" })),
                    ("intervalMs", json!({ "type": "number", "description": "轮询间隔，默认 200" })),
                ],
            )),
        ),
        Tool::new(
            "ui_tap",
            "点击 → 等稳定 → 与点击前比对，一次返回变化的点坐标。验证交互是否生效用它。",
            schema(merge(
                &{
                    let mut props = vec![
                        ("x", json!({ "type": "number" })),
                        ("y", json!({ "type": "number" })),
                    ];
                    props.extend(target_properties());
                    props.extend(wait_properties());
                    props
                },
                &{
                    let mut extra = vec![
                        ("settleMs", json!({ "type": "number", "description": "点击后先等的时间，默认 120" })),
                        ("button", json!({ "type": "string", "enum": ["left", "right", "middle"] })),
                        ("count", json!({ "type": "number" })),
                        ("includeImage", json!({ "type": "boolean", "description": "true 时附带变化后的截图" })),
                        ("maxPx", json!({ "type": "number" })),
                    ];
                    // tap 整段独占租约，因此同样接受互斥参数。
                    extra.extend(lease_properties());
                    extra
                },
            )),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_object_schema() {
        let tools = tool_list();
        assert_eq!(tools.len(), 22);
        for tool in &tools {
            assert!(tool.description.is_some(), "{} 缺少描述", tool.name);
            assert_eq!(
                tool.input_schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{} 的 schema 不是 object",
                tool.name
            );
            assert!(
                tool.input_schema.contains_key("properties"),
                "{} 缺少 properties",
                tool.name
            );
        }
    }

    #[test]
    fn required_fields_are_declared() {
        let tools = tool_list();
        let find = |name: &str| {
            tools
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
        };
        let required = |tool: &Tool| -> Vec<String> {
            tool.input_schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default()
        };

        assert_eq!(required(find("ui_pixel")), vec!["shot", "points"]);
        assert_eq!(required(find("ui_diff")), vec!["before", "after"]);
        assert_eq!(required(find("ui_click")), vec!["x", "y"]);
        assert_eq!(required(find("ui_actions")), vec!["path"]);
        assert_eq!(required(find("ui_press")), vec!["path"]);
        assert_eq!(required(find("ui_set_value")), vec!["path", "value"]);
        assert!(required(find("ui_doctor")).is_empty());
    }
}
