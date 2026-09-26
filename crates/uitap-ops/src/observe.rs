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

/// 应用最前的普通窗口（窗口层级 0、叠放序号最小）。
/// 给「按应用名取一块画面」用：窗口 id 每次重启都会变，按应用名取就不必先列窗口。
pub fn app_window(backend: &dyn Backend, app: &str) -> OpResult<uitap_core::backend::WindowInfo> {
    let windows = backend.windows(false).map_err(|e| e.to_string())?;
    pick_app_window(&windows, app).ok_or_else(|| {
        format!(
            "没找到「{app}」的普通窗口（层级 0）：名字对不上，或它当前没有窗口。\
             先列一次窗口看实际的名字"
        )
    })
}

/// 从窗口列表里挑出目标应用最前的普通窗口。
fn pick_app_window(
    windows: &[uitap_core::backend::WindowInfo],
    app: &str,
) -> Option<uitap_core::backend::WindowInfo> {
    let needle = app.to_ascii_lowercase();
    windows
        .iter()
        .filter(|w| w.layer == 0 && w.app.to_ascii_lowercase().contains(&needle))
        .min_by_key(|w| w.z)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uitap_core::backend::WindowInfo;
    use uitap_core::geom::Rect;

    fn window(id: u64, app: &str, layer: i32, z: usize) -> WindowInfo {
        WindowInfo {
            id,
            pid: 1,
            app: app.to_string(),
            title: String::new(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            layer,
            onscreen: true,
            z,
        }
    }

    #[test]
    fn picks_frontmost_normal_window_of_the_app() {
        let windows = vec![
            window(1, "classroom_app", 25, 0),
            window(2, "classroom_app", 0, 3),
            window(3, "classroom_app", 0, 1),
        ];
        assert_eq!(pick_app_window(&windows, "classroom").map(|w| w.id), Some(3));
    }

    #[test]
    fn ignores_other_apps_and_non_normal_layers() {
        let windows = vec![
            window(1, "other_app", 0, 0),
            window(2, "classroom_app", 25, 0),
        ];
        assert!(pick_app_window(&windows, "classroom").is_none());
    }

    #[test]
    fn app_matching_is_case_insensitive_substring() {
        let windows = vec![window(9, "Classroom_App", 0, 5)];
        assert_eq!(pick_app_window(&windows, "CLASSROOM").map(|w| w.id), Some(9));
    }
}
