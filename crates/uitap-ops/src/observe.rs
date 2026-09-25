//! 观测类操作：授权自检、显示器、窗口、前台应用。

use serde_json::{Map, Value};

use uitap_core::backend::Backend;
use uitap_core::store;

use crate::json::{app_json, window_json};
use crate::types::{OpResult, WindowQuery};

/// `capture_available` 由入口层传入：它是平台能力，不属于操作层。
pub fn doctor(backend: &dyn Backend, capture_available: bool) -> Value {
    let permissions = backend.permissions();
    let mut hints: Vec<Value> = Vec::new();
    if !permissions.screen_recording {
        hints.push(Value::String(
            "屏幕录制未授权：系统设置 → 隐私与安全性 → 屏幕录制，勾选承载本工具的进程".into(),
        ));
    }
    if !permissions.accessibility {
        hints.push(Value::String(
            "辅助功能未授权：系统设置 → 隐私与安全性 → 辅助功能，勾选同一进程".into(),
        ));
    }

    let mut map = Map::new();
    map.insert(
        "screenRecording".into(),
        Value::Bool(permissions.screen_recording),
    );
    map.insert("accessibility".into(), Value::Bool(permissions.accessibility));
    map.insert(
        "screencapture".into(),
        Value::Bool(capture_available),
    );
    map.insert(
        "shotDirectory".into(),
        Value::String(store::shot_dir().to_string_lossy().into_owned()),
    );
    if !hints.is_empty() {
        map.insert("hints".into(), Value::Array(hints));
    }
    Value::Object(map)
}

pub fn screens(backend: &dyn Backend) -> OpResult<Value> {
    let displays = backend.displays().map_err(|e| e.to_string())?;
    let main_index = displays.iter().position(|d| d.main);

    let list: Vec<Value> = displays
        .iter()
        .map(|d| {
            let mut map = Map::new();
            map.insert("index".into(), Value::from(d.index));
            map.insert("id".into(), Value::from(d.id));
            map.insert("bounds".into(), uitap_core::jsonout::rect_json(d.bounds));
            map.insert(
                "pixels".into(),
                uitap_core::jsonout::size_json(d.pixel_width, d.pixel_height),
            );
            map.insert("scale".into(), uitap_core::jsonout::num(d.scale()));
            if d.main {
                map.insert("main".into(), Value::Bool(true));
            }
            Value::Object(map)
        })
        .collect();

    let mut map = Map::new();
    map.insert("displays".into(), Value::Array(list));
    if let Some(index) = main_index {
        map.insert("mainIndex".into(), Value::from(index));
    }
    Ok(Value::Object(map))
}

pub fn windows(backend: &dyn Backend, query: &WindowQuery) -> OpResult<Value> {
    let mut windows = backend
        .windows(query.all)
        .map_err(|e| e.to_string())?;

    if let Some(app) = &query.app {
        let needle = app.to_ascii_lowercase();
        windows.retain(|w| w.app.to_ascii_lowercase().contains(&needle));
    }
    if let Some(title) = &query.title {
        let needle = title.to_ascii_lowercase();
        windows.retain(|w| w.title.to_ascii_lowercase().contains(&needle));
    }
    if let Some(layer) = query.layer {
        windows.retain(|w| w.layer == layer);
    }
    if query.min_width > 0.0 {
        windows.retain(|w| w.bounds.w >= query.min_width);
    }
    if query.min_height > 0.0 {
        windows.retain(|w| w.bounds.h >= query.min_height);
    }
    if query.front_only {
        if let Ok(front) = backend.frontmost() {
            windows.retain(|w| w.pid == front.pid);
        }
    }

    // 大窗口优先，同尺寸时最前的在前。
    windows.sort_by(uitap_core::backend::WindowInfo::ordered);
    if let Some(limit) = query.limit {
        windows.truncate(limit);
    }

    let front_pid = backend.frontmost().map(|f| f.pid).unwrap_or(0);
    let list: Vec<Value> = windows
        .iter()
        .enumerate()
        .map(|(index, w)| window_json(w, front_pid, index))
        .collect();

    let mut map = Map::new();
    map.insert("count".into(), Value::from(list.len()));
    map.insert("windows".into(), Value::Array(list));
    Ok(Value::Object(map))
}

pub fn frontmost(backend: &dyn Backend) -> OpResult<Value> {
    let app = backend.frontmost().map_err(|e| e.to_string())?;
    Ok(app_json(&app))
}
