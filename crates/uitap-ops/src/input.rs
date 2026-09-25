//! 输入类操作：指针、键盘、应用激活。

use serde_json::{Map, Value};

use uitap_core::backend::{ActivateTarget, Backend, MouseButton, Modifier};
use uitap_core::geom::Point;

use crate::json::ok_at_json;
use crate::types::{ActivateRequest, OpResult, ScrollRequest};

pub fn click(
    backend: &dyn Backend,
    at: Point,
    button: MouseButton,
    count: u32,
) -> OpResult<Value> {
    let count = count.max(1);
    backend
        .click(at, button, count)
        .map_err(|e| e.to_string())?;

    let mut map = match ok_at_json(at) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("button".into(), Value::String(button.as_str().into()));
    map.insert("count".into(), Value::from(count));
    Ok(Value::Object(map))
}

pub fn move_to(backend: &dyn Backend, at: Point) -> OpResult<Value> {
    backend.move_to(at).map_err(|e| e.to_string())?;
    Ok(ok_at_json(at))
}

pub fn drag(
    backend: &dyn Backend,
    from: Point,
    to: Point,
    button: MouseButton,
    duration_ms: u64,
) -> OpResult<Value> {
    backend
        .drag(from, to, button, duration_ms)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("from".into(), uitap_core::jsonout::point_json(from));
    map.insert("to".into(), uitap_core::jsonout::point_json(to));
    Ok(Value::Object(map))
}

pub fn scroll(backend: &dyn Backend, request: &ScrollRequest) -> OpResult<Value> {
    if request.dx == 0 && request.dy == 0 {
        return Err("--dy or --dx is required".into());
    }
    backend
        .scroll(request.at, request.dx, request.dy)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("dx".into(), Value::from(request.dx));
    map.insert("dy".into(), Value::from(request.dy));
    Ok(Value::Object(map))
}

pub fn type_text(backend: &dyn Backend, text: &str, delay_ms: u64) -> OpResult<Value> {
    backend
        .type_text(text, delay_ms)
        .map_err(|e| e.to_string())?;

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("length".into(), Value::from(text.chars().count()));
    map.insert("delayMs".into(), Value::from(delay_ms));
    Ok(Value::Object(map))
}

pub fn key(
    backend: &dyn Backend,
    combo: &str,
    key_code: u16,
    modifiers: &[Modifier],
    repeat: u32,
) -> OpResult<Value> {
    let repeat = repeat.max(1);
    for index in 0..repeat {
        backend
            .key(key_code, modifiers)
            .map_err(|e| e.to_string())?;
        if index < repeat - 1 {
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }

    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("combo".into(), Value::String(combo.to_string()));
    map.insert("keyCode".into(), Value::from(key_code));
    map.insert("repeat".into(), Value::from(repeat));
    Ok(Value::Object(map))
}

pub fn activate(backend: &dyn Backend, request: &ActivateRequest) -> OpResult<Value> {
    let target = if let Some(pid) = request.pid {
        ActivateTarget::Pid(pid)
    } else if let Some(window) = request.window {
        ActivateTarget::Window(window)
    } else if let Some(app) = &request.app {
        ActivateTarget::App(app.clone())
    } else {
        return Err("--app, --pid or --window is required".into());
    };

    let app = backend.activate(&target).map_err(|e| e.to_string())?;

    // 与既有实现一致：activate 只回 app / pid / frontmost，不带 bundleId。
    let mut map = Map::new();
    map.insert("ok".into(), Value::Bool(true));
    map.insert("app".into(), Value::String(app.app));
    map.insert("pid".into(), Value::from(app.pid));
    map.insert("frontmost".into(), Value::Bool(app.frontmost));
    Ok(Value::Object(map))
}
