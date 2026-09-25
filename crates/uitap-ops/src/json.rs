//! JSON 契约的构造。CLI 与 MCP 共用同一份，避免两边漂移。

use serde_json::{Map, Value};

use uitap_core::backend::{RawCapture, RunningApp, WindowInfo};
use uitap_core::geom::{Anchor, Point, Rect};
use uitap_core::jsonout::{num, point_json, ratio, rect_json, size_json};
use uitap_core::pixels::DiffResult;

pub fn error_value(message: impl AsRef<str>) -> Value {
    let mut map = Map::new();
    map.insert("error".into(), Value::String(message.as_ref().into()));
    Value::Object(map)
}

pub fn shot_json(capture: &RawCapture) -> Value {
    let mut map = Map::new();
    map.insert(
        "path".into(),
        Value::String(capture.path.to_string_lossy().into_owned()),
    );
    map.insert(
        "size".into(),
        size_json(capture.pixel_width, capture.pixel_height),
    );
    map.insert("origin".into(), point_json(capture.origin));
    map.insert("scale".into(), num(capture.scale()));
    map.insert("mode".into(), Value::String(capture.mode.as_str().into()));
    if let Some(id) = capture.window_id {
        map.insert("window".into(), Value::from(id));
    }
    Value::Object(map)
}

pub fn window_json(w: &WindowInfo, front_pid: i32, index: usize) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), Value::from(w.id));
    map.insert("pid".into(), Value::from(w.pid));
    map.insert("app".into(), Value::String(w.app.clone()));
    map.insert("bounds".into(), rect_json(w.bounds));
    map.insert("layer".into(), Value::from(w.layer));
    map.insert("index".into(), Value::from(index));
    map.insert("z".into(), Value::from(w.z));
    if !w.title.is_empty() {
        map.insert("title".into(), Value::String(clip_title(&w.title, 80)));
    }
    if !w.onscreen {
        map.insert("onscreen".into(), Value::Bool(false));
    }
    if w.pid == front_pid {
        map.insert("front".into(), Value::Bool(true));
    }
    Value::Object(map)
}

pub fn app_json(app: &RunningApp) -> Value {
    let mut map = Map::new();
    map.insert("app".into(), Value::String(app.app.clone()));
    map.insert("pid".into(), Value::from(app.pid));
    map.insert("bundleId".into(), Value::String(app.bundle_id.clone()));
    Value::Object(map)
}

/// 差分结果。`anchor` 与 `use_point` 决定区域与包围盒按哪种坐标输出。
pub fn diff_json(result: &DiffResult, anchor: Option<Anchor>, use_point: bool) -> Value {
    let convert = |r: Rect| -> Rect {
        match (use_point, anchor) {
            (true, Some(anchor)) => anchor.to_point(r),
            _ => r,
        }
    };

    let mut map = Map::new();
    map.insert("changed".into(), Value::Bool(result.changed()));
    map.insert("ratio".into(), ratio(result.ratio()));
    map.insert("changedPixels".into(), Value::from(result.changed_pixels));
    map.insert(
        "units".into(),
        Value::String(if use_point { "point" } else { "pixel" }.into()),
    );
    map.insert("size".into(), size_json(result.width, result.height));
    if let Some(bounds) = result.bounds {
        map.insert("bounds".into(), rect_json(convert(bounds)));
    }
    map.insert(
        "regions".into(),
        Value::Array(
            result
                .regions
                .iter()
                .map(|region| {
                    let mut item = match rect_json(convert(region.rect)) {
                        Value::Object(map) => map,
                        _ => Map::new(),
                    };
                    item.insert("pixels".into(), Value::from(region.pixels));
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    Value::Object(map)
}

pub fn ok_at_json(at: Point) -> Value {
    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("at".into(), point_json(at));
    Value::Object(map)
}

fn clip_title(title: &str, limit: usize) -> String {
    if title.chars().count() <= limit {
        return title.to_string();
    }
    let clipped: String = title.chars().take(limit).collect();
    format!("{clipped}…")
}
