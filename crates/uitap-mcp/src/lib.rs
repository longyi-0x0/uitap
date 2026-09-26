//! MCP server：把操作层包成模型可直接调用的工具。
//!
//! 与既有契约保持一致的两点：截图默认只回锚点与短 id（不带图像，省 token），
//! 以及错误以工具级内容返回而不是协议错误（调用方能看到可操作的信息）。

mod extract;
mod tools;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{Map, Value};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer, ServiceExt};
use rmcp::{ErrorData, ServerHandler};

use uitap_core::backend::{CaptureTarget, ElementAction, TreeLimits};
use uitap_core::geom::Rect;
use uitap_ops::{
    ax, image, input, lease, observe, tap as tap_op, wait, ActivateRequest, AnchorOverride,
    AppTarget, CropRequest, DiffRequest, ElementQuery, FindPixelsRequest, LeaseSettings,
    PixelRequest, ScrollRequest, ShotRequest, TapRequest, Units, WaitParams, WindowQuery,
};
use uitap_platform::{capture_available, open, parse_combo, Current};

/// 最近的截图登记表：短 id 代替长路径，省 token。进程生命周期内有效。
#[derive(Default)]
struct ShotRegistry {
    next: u64,
    paths: HashMap<String, PathBuf>,
}

impl ShotRegistry {
    fn register(&mut self, path: &Path) -> String {
        self.next += 1;
        let id = format!("s{}", self.next);
        self.paths.insert(id.clone(), path.to_path_buf());
        id
    }

    /// 截图 id 或文件路径都能用。未知 id 原样返回，让下层报出「文件不存在」。
    fn resolve(&self, reference: &str) -> PathBuf {
        match self.paths.get(reference) {
            Some(path) => path.clone(),
            None => PathBuf::from(reference),
        }
    }
}

struct UitapServer {
    shots: Arc<Mutex<ShotRegistry>>,
}

impl UitapServer {
    fn new() -> Self {
        Self {
            shots: Arc::new(Mutex::new(ShotRegistry::default())),
        }
    }
}

impl ServerHandler for UitapServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            rmcp::model::Implementation::new("uitap", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let shots = self.shots.clone();
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);

        // 操作层会起子进程、sleep 与读图，放到阻塞线程池里，别占住异步执行器。
        let outcome = tokio::task::spawn_blocking(move || dispatch(&shots, &name, &args))
            .await
            .map_err(|e| ErrorData::internal_error(format!("tool task failed: {e}"), None))?;

        Ok(match outcome {
            Ok(blocks) => CallToolResponse::from(CallToolResult::success(blocks)),
            Err(message) => CallToolResponse::from(CallToolResult::error(vec![ContentBlock::text(
                format!("错误：{message}"),
            )])),
        })
    }
}

/// 起 stdio MCP server，直到客户端断开。
pub async fn serve() -> anyhow::Result<()> {
    let service = UitapServer::new()
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("initialize failed: {e}"))?;
    service.waiting().await?;
    Ok(())
}

// ---------------------------------------------------------------- 工具分发

fn dispatch(
    shots: &Arc<Mutex<ShotRegistry>>,
    name: &str,
    args: &Value,
) -> Result<Vec<ContentBlock>, String> {
    let backend: Current = open();
    match name {
        "ui_doctor" => Ok(vec![text(observe::doctor(&backend, capture_available()))]),

        "ui_screens" => Ok(vec![text(observe::screens(&backend)?)]),

        "ui_windows" => {
            let query = WindowQuery {
                app: extract::string(args, "app"),
                title: extract::string(args, "title"),
                all: extract::flag(args, "all"),
                front_only: extract::flag(args, "frontOnly"),
                min_width: extract::number(args, "minWidth").unwrap_or(0.0),
                min_height: extract::number(args, "minHeight").unwrap_or(0.0),
                layer: extract::integer(args, "layer").map(|v| v as i32),
                limit: Some(
                    extract::integer(args, "limit")
                        .filter(|v| *v >= 0)
                        .unwrap_or(25) as usize,
                ),
            };
            Ok(vec![text(observe::windows(&backend, &query)?)])
        }

        "ui_shot" => {
            let request = ShotRequest {
                target: capture_target(&backend, args)?,
                path: extract::string(args, "path").map(PathBuf::from),
                max_px: extract::integer(args, "maxPx").map(|v| v.max(0) as usize),
                tag: "shot".to_string(),
            };
            let outcome = image::shot(&backend, &request)?;
            let mut payload = outcome.json();
            let id = shots
                .lock()
                .map_err(|_| "截图登记表不可用".to_string())?
                .register(outcome.path());
            if let Value::Object(map) = &mut payload {
                map.insert("id".into(), Value::String(id));
            }

            let mut blocks = vec![text(payload)];
            if extract::flag(args, "includeImage") {
                let max_px = extract::integer(args, "maxPx").unwrap_or(1280) as usize;
                let (_, block) = image_block(outcome.path(), None, max_px)?;
                blocks.push(block);
            }
            Ok(blocks)
        }

        "ui_zoom" => {
            let source = resolve(shots, args, "shot")?;
            let max_px = extract::integer(args, "maxPx").unwrap_or(1400).max(1) as usize;
            let region = extract::rect(args, "region");
            let (meta, block) = image_block(&source, region, max_px)?;
            Ok(vec![text(meta), block])
        }

        "ui_pixel" => {
            let request = PixelRequest {
                path: resolve(shots, args, "shot")?,
                points: extract::points(args, "points"),
                // 单位交给操作层定：有锚点按点、没有按像素，并在返回里说明。
                units: None,
                anchor_override: AnchorOverride::default(),
            };
            let mut payload = image::pixel(&request)?;

            // 期望色断言在服务端完成，模型只需读布尔值。
            if let Some(expect) = args.get("expect").and_then(Value::as_array) {
                let tolerance = extract::number(args, "tolerance").unwrap_or(12.0);
                annotate_matches(&mut payload, expect, tolerance);
            }
            Ok(vec![text(payload)])
        }

        "ui_find_pixels" => {
            let mut colors: Vec<[u8; 3]> = args
                .get("colors")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(image::parse_color).collect())
                .unwrap_or_default();
            if let Some(color) = args.get("color").and_then(image::parse_color) {
                colors.push(color);
            }
            let request = FindPixelsRequest {
                path: resolve(shots, args, "shot")?,
                region: extract::rect(args, "region"),
                units: None,
                colors,
                tolerance: extract::number(args, "tolerance").unwrap_or(12.0),
                min_pixels: extract::integer(args, "minPixels").unwrap_or(4).max(1) as usize,
                max_clusters: extract::integer(args, "maxClusters").unwrap_or(8).max(1) as usize,
                anchor_override: AnchorOverride::default(),
            };
            Ok(vec![text(image::find_pixels(&request)?)])
        }

        "ui_diff" => {
            let mut request = DiffRequest::new(resolve(shots, args, "before")?, resolve(shots, args, "after")?);
            request.threshold = extract::integer(args, "threshold").map(|v| v as i32);
            request.min_pixels = extract::integer(args, "minPixels").map(|v| v.max(0) as usize);
            request.max_regions = extract::integer(args, "maxRegions").map(|v| v.max(0) as usize);
            request.region = extract::rect(args, "region");
            // 单位交给操作层定：有锚点按点、没有按像素，并在返回里说明。
            request.units = None;
            Ok(vec![text(image::diff(&request)?)])
        }

        "ui_wait_stable" => {
            let params = wait_params(args);
            Ok(vec![text(wait::wait_stable_json(
                &backend,
                &capture_target(&backend, args)?,
                &params,
            )?)])
        }

        "ui_click" => {
            let point = uitap_core::geom::Point::new(
                extract::required_number(args, "x")?,
                extract::required_number(args, "y")?,
            );
            let count = extract::integer(args, "count").unwrap_or(1).max(1) as u32;
            Ok(vec![text(input::click(
                &backend,
                point,
                extract::button(args),
                count,
                &lease_settings(args),
            )?)])
        }

        "ui_drag" => {
            let from = extract::required_point(args, "from")?;
            let to = extract::required_point(args, "to")?;
            let duration = extract::integer(args, "durationMs").unwrap_or(300).max(0) as u64;
            Ok(vec![text(input::drag(
                &backend,
                from,
                to,
                extract::button(args),
                duration,
                &lease_settings(args),
            )?)])
        }

        "ui_scroll" => {
            let request = ScrollRequest {
                at: extract::point(args, "at"),
                dx: extract::integer(args, "dx").unwrap_or(0) as i32,
                dy: extract::integer(args, "dy").unwrap_or(0) as i32,
            };
            Ok(vec![text(input::scroll(
                &backend,
                &request,
                &lease_settings(args),
            )?)])
        }

        "ui_type" => {
            let text_value = extract::required_string(args, "text")?;
            let delay = extract::integer(args, "delayMs").unwrap_or(0).max(0) as u64;
            Ok(vec![text(input::type_text(
                &backend,
                &text_value,
                delay,
                &lease_settings(args),
            )?)])
        }

        "ui_key" => {
            let combo = extract::required_string(args, "combo")?;
            let (key_code, modifiers, _name) = parse_combo(&combo)
                .ok_or_else(|| format!("unrecognized combo: {combo}"))?;
            let repeat = extract::integer(args, "repeat").unwrap_or(1).max(1) as u32;
            Ok(vec![text(input::key(
                &backend,
                &combo,
                key_code,
                &modifiers,
                repeat,
                &lease_settings(args),
            )?)])
        }

        "ui_activate" => {
            let request = ActivateRequest {
                app: extract::string(args, "app"),
                pid: extract::integer(args, "pid").map(|v| v as i32),
                window: extract::integer(args, "window").map(|v| v as u64),
            };
            Ok(vec![text(input::activate(
                &backend,
                &request,
                &lease_settings(args),
            )?)])
        }

        "ui_tree" => {
            let pid = element_pid(&backend, args)?;
            Ok(vec![text(ax::tree(
                &backend,
                pid,
                &tree_limits(args, TREE_DEFAULT_NODES),
                element_window(args),
            )?)])
        }

        "ui_find" => {
            let pid = element_pid(&backend, args)?;
            Ok(vec![text(ax::find(
                &backend,
                pid,
                &element_query(args),
                &tree_limits(args, TreeLimits::default().max_nodes),
                element_window(args),
                extract::integer(args, "limit").unwrap_or(20).max(1) as usize,
            )?)])
        }

        "ui_actions" => {
            let pid = element_pid(&backend, args)?;
            let path = extract::required_string(args, "path")?;
            Ok(vec![text(ax::actions(&backend, pid, &path)?)])
        }

        "ui_press" => {
            let pid = element_pid(&backend, args)?;
            let path = extract::required_string(args, "path")?;
            let action = extract::string(args, "action")
                .and_then(|name| ElementAction::parse(&name))
                .unwrap_or(ElementAction::Press);
            Ok(vec![text(ax::act(&backend, pid, &path, action)?)])
        }

        "ui_set_value" => {
            let pid = element_pid(&backend, args)?;
            let path = extract::required_string(args, "path")?;
            let value = extract::required_string(args, "value")?;
            Ok(vec![text(ax::set_value(&backend, pid, &path, &value)?)])
        }

        "ui_wait_for" => {
            let pid = element_pid(&backend, args)?;
            Ok(vec![text(ax::wait_for(
                &backend,
                pid,
                &element_query(args),
                &tree_limits(args, TreeLimits::default().max_nodes),
                element_window(args),
                extract::integer(args, "timeoutMs").unwrap_or(5000).max(100) as u64,
                extract::integer(args, "intervalMs").unwrap_or(200).max(50) as u64,
            )?)])
        }

        "ui_lease" => {
            let action = extract::string(args, "action").unwrap_or_else(|| "status".to_string());
            Ok(vec![text(lease::lease_report(&action)?)])
        }

        "ui_tap" => {
            let at = uitap_core::geom::Point::new(
                extract::required_number(args, "x")?,
                extract::required_number(args, "y")?,
            );
            let request = TapRequest {
                at,
                target: capture_target(&backend, args)?,
                button: extract::button(args),
                count: extract::integer(args, "count").unwrap_or(1).max(1) as u32,
                settle_ms: extract::integer(args, "settleMs").unwrap_or(120).max(0) as u64,
                wait: wait_params(args),
                units: Some(Units::Point),
                keep: false,
                lease: lease_settings(args),
            };
            let payload = tap_op::tap(&backend, &request)?;

            if extract::flag(args, "includeImage") {
                let after = image::shot(&backend, &ShotRequest {
                    target: request.target.clone(),
                    path: None,
                    max_px: None,
                    tag: "tap-after".to_string(),
                })?;
                let max_px = extract::integer(args, "maxPx").unwrap_or(1280).max(1) as usize;
                let (_, block) = image_block(after.path(), None, max_px)?;
                return Ok(vec![text(payload), block]);
            }
            Ok(vec![text(payload)])
        }

        other => Err(format!("未知工具：{other}")),
    }
}

// ---------------------------------------------------------------- 辅助

fn text(value: Value) -> ContentBlock {
    ContentBlock::text(compact(&value).to_string())
}

/// 截图目标：window / region / display 显式给出时按它们；只给 app 时取该应用最前的普通窗口。
fn capture_target(backend: &Current, args: &Value) -> Result<CaptureTarget, String> {
    if extract::integer(args, "window").is_none()
        && extract::rect(args, "region").is_none()
        && extract::integer(args, "display").is_none()
    {
        if let Some(app) = extract::string(args, "app") {
            return Ok(CaptureTarget::Window(observe::app_window(backend, &app)?.id));
        }
    }
    target(args)
}

fn target(args: &Value) -> Result<CaptureTarget, String> {
    if let Some(window) = extract::integer(args, "window") {
        return Ok(CaptureTarget::Window(window as u64));
    }
    if let Some(region) = extract::rect(args, "region") {
        return Ok(CaptureTarget::Region(region));
    }
    if let Some(display) = extract::integer(args, "display") {
        return Ok(CaptureTarget::Screen {
            display_index: Some(display.max(0) as usize),
        });
    }
    Ok(CaptureTarget::Screen {
        display_index: None,
    })
}

/// 元素类工具的目标进程。
fn element_pid(backend: &Current, args: &Value) -> Result<i32, String> {
    let target = AppTarget::from_parts(
        extract::string(args, "app"),
        extract::integer(args, "pid").map(|v| v as i32),
        extract::integer(args, "window").map(|v| v as u64),
    )
    .ok_or_else(|| "app、pid 或 window 至少给一个，用来确定目标应用".to_string())?;
    ax::resolve_pid(backend, &target)
}

/// 元素树遍历的边界，沿用 core 的默认值；`ui_tree` 会把整棵树当结果返回，默认收得更小。
fn tree_limits(args: &Value, default_nodes: usize) -> TreeLimits {
    let defaults = TreeLimits::default();
    TreeLimits {
        max_depth: extract::integer(args, "depth")
            .unwrap_or(defaults.max_depth as i64)
            .max(0) as usize,
        max_nodes: extract::integer(args, "maxNodes")
            .unwrap_or(default_nodes as i64)
            .max(1) as usize,
    }
}

/// `ui_tree` 默认的节点数上限：节点按字节算不便宜，默认只取一层能看清的量。
const TREE_DEFAULT_NODES: usize = 40;

/// `--window` 限定元素树范围；给 pid 或 app 时不限定。
fn element_window(args: &Value) -> Option<u64> {
    if args.get("pid").is_some() || args.get("app").is_some() {
        return None;
    }
    extract::integer(args, "window").map(|v| v as u64)
}

fn element_query(args: &Value) -> ElementQuery {
    ElementQuery {
        role: extract::string(args, "role"),
        subrole: extract::string(args, "subrole"),
        title: extract::string(args, "title"),
        value: extract::string(args, "value"),
        identifier: extract::string(args, "identifier"),
        enabled_only: extract::flag(args, "enabled"),
    }
}

/// 输入操作的互斥参数。默认开启；`noLock` 关闭，`waitMs` 调整等待预算。
fn lease_settings(args: &Value) -> LeaseSettings {
    let settings = LeaseSettings::default()
        .with_wait(extract::integer(args, "waitMs").unwrap_or(5_000).max(0) as u64);
    if extract::flag(args, "noLock") {
        settings.without_lock()
    } else {
        settings
    }
}

fn wait_params(args: &Value) -> WaitParams {
    WaitParams {
        interval_ms: extract::integer(args, "intervalMs").unwrap_or(120).max(40) as u64,
        timeout_ms: extract::integer(args, "timeoutMs").unwrap_or(4000).max(200) as u64,
        ratio_threshold: extract::number(args, "threshold").unwrap_or(0.0006),
        stable_samples: extract::integer(args, "stableSamples").unwrap_or(2).max(1) as usize,
    }
}

fn resolve(
    shots: &Arc<Mutex<ShotRegistry>>,
    args: &Value,
    key: &str,
) -> Result<PathBuf, String> {
    let reference = extract::required_string(args, key)?;
    let registry = shots.lock().map_err(|_| "截图登记表不可用".to_string())?;
    Ok(registry.resolve(&reference))
}

/// 生成给模型看的缩放副本，不动原图，因此像素换算仍基于原图。
/// 同时回一份元数据（裁剪后的 origin/scale、这次 region 用的坐标），让模型不必自己换算。
fn image_block(
    source: &Path,
    region_points: Option<Rect>,
    max_px: usize,
) -> Result<(Value, ContentBlock), String> {
    let output = PathBuf::from(format!(
        "{}-view.png",
        source.to_string_lossy().trim_end_matches(".png")
    ));

    let request = CropRequest {
        input: source.to_path_buf(),
        output: Some(output.clone()),
        region: None,
        region_points,
        max_px: Some(max_px),
        anchor_override: AnchorOverride::default(),
    };
    let meta = image::crop(&request)?;

    let bytes = std::fs::read(&output).map_err(|e| format!("cannot read {}: {e}", output.display()))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok((meta, ContentBlock::image(encoded, "image/png")))
}

/// 期望色断言：给每点加 `match`，并汇总 `allMatch`。
fn annotate_matches(payload: &mut Value, expect: &[Value], tolerance: f64) {
    let Some(points) = payload.get_mut("points").and_then(Value::as_array_mut) else {
        return;
    };

    let mut compared = 0usize;
    let mut matched = 0usize;
    for (index, point) in points.iter_mut().enumerate() {
        let Some(target) = expect.get(index).and_then(parse_hex) else {
            continue;
        };
        let Some(actual) = point.get("hex").and_then(parse_hex) else {
            continue;
        };
        let is_match = actual
            .iter()
            .zip(target.iter())
            .all(|(a, b)| (*a as f64 - *b as f64).abs() <= tolerance);
        if let Value::Object(map) = point {
            map.insert("match".into(), Value::Bool(is_match));
        }
        compared += 1;
        if is_match {
            matched += 1;
        }
    }

    if compared > 0 {
        if let Value::Object(map) = payload {
            map.insert("allMatch".into(), Value::Bool(matched == compared));
        }
    }
}

fn parse_hex(value: &Value) -> Option<[u8; 3]> {
    image::parse_color(value)
}

/// 去掉空串、null 与空数组，压低返回体积。布尔值保留，避免语义丢失。
fn compact(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(compact).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, raw) in map {
                let pruned = compact(raw);
                let drop = match &pruned {
                    Value::String(s) => s.is_empty(),
                    Value::Null => true,
                    Value::Array(items) => items.is_empty(),
                    _ => false,
                };
                if !drop {
                    out.insert(key.clone(), pruned);
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_registered_shot_id() {
        let mut registry = ShotRegistry::default();
        let path = PathBuf::from("/tmp/uitap/a.png");
        let id = registry.register(&path);
        assert_eq!(id, "s1");
        assert_eq!(registry.resolve("s1"), path);
        // 未知引用原样返回，让下层报「文件不存在」。
        assert_eq!(registry.resolve("/other/b.png"), PathBuf::from("/other/b.png"));
    }

    #[test]
    fn compact_drops_empty_values() {
        let value = json!({
            "keep": 1,
            "emptyString": "",
            "nullValue": null,
            "emptyArray": [],
            "nested": { "ok": true, "drop": [] },
            "falseKept": false,
        });
        let pruned = compact(&value);
        assert_eq!(
            pruned,
            json!({ "keep": 1, "nested": { "ok": true }, "falseKept": false })
        );
    }

    #[test]
    fn parses_hex_and_rgb_expectations() {
        assert_eq!(parse_hex(&json!("#FF8000")), Some([255, 128, 0]));
        assert_eq!(parse_hex(&json!([1, 2, 3])), Some([1, 2, 3]));
        assert_eq!(parse_hex(&json!("xyz")), None);
        // 越界通道被夹到 0..255
        assert_eq!(parse_hex(&json!([-5, 300, 10.6])), Some([0, 255, 11]));
    }

    #[test]
    fn annotates_match_flags() {
        let mut payload = json!({
            "units": "point",
            "points": [
                { "x": 5, "y": 5, "hex": "#1D1B24" },
                { "x": 100, "y": 100, "hex": "#131414" },
            ],
        });
        let expect = vec![json!("#1D1B24"), json!("#FFFFFF")];
        annotate_matches(&mut payload, &expect, 12.0);

        assert_eq!(payload["points"][0]["match"], json!(true));
        assert_eq!(payload["points"][1]["match"], json!(false));
        assert_eq!(payload["allMatch"], json!(false));
    }

    #[test]
    fn tolerance_is_inclusive() {
        let mut payload = json!({
            "points": [{ "x": 0, "y": 0, "hex": "#0A0A0A" }],
        });
        annotate_matches(&mut payload, &vec![json!("#000000")], 10.0);
        assert_eq!(payload["points"][0]["match"], json!(true));
        assert_eq!(payload["allMatch"], json!(true));
    }
}
