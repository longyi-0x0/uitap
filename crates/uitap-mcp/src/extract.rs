//! MCP 工具的入参提取。
//!
//! rmcp 的 `arguments` 是通用 JSON 对象；这里集中做取值与校验，让工具实现保持直线。

use serde_json::Value;

use uitap_core::backend::MouseButton;
use uitap_core::geom::{Point, Rect};


pub fn string(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn required_string(args: &Value, key: &str) -> Result<String, String> {
    string(args, key).ok_or_else(|| format!("{key} 必填"))
}

pub fn number(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

pub fn required_number(args: &Value, key: &str) -> Result<f64, String> {
    number(args, key).ok_or_else(|| format!("{key} 必填且必须是数字"))
}

pub fn integer(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

pub fn boolean(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

pub fn flag(args: &Value, key: &str) -> bool {
    boolean(args, key).unwrap_or(false)
}

/// `[x, y]` 或 `{"x":..,"y":..}`。
pub fn point(args: &Value, key: &str) -> Option<Point> {
    let value = args.get(key)?;
    if let Some(array) = value.as_array() {
        if array.len() >= 2 {
            return Some(Point::new(array[0].as_f64()?, array[1].as_f64()?));
        }
        return None;
    }
    Some(Point::new(value.get("x")?.as_f64()?, value.get("y")?.as_f64()?))
}

pub fn required_point(args: &Value, key: &str) -> Result<Point, String> {
    point(args, key).ok_or_else(|| format!("{key} 必填，形如 [x, y]"))
}

pub fn rect(args: &Value, key: &str) -> Option<Rect> {
    let array = args.get(key)?.as_array()?;
    if array.len() < 4 {
        return None;
    }
    Some(Rect::new(
        array[0].as_f64()?,
        array[1].as_f64()?,
        array[2].as_f64()?,
        array[3].as_f64()?,
    ))
}

pub fn points(args: &Value, key: &str) -> Vec<Point> {
    let Some(array) = args.get(key).and_then(Value::as_array) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|entry| {
            if let Some(inner) = entry.as_array() {
                if inner.len() >= 2 {
                    return Some(Point::new(inner[0].as_f64()?, inner[1].as_f64()?));
                }
                return None;
            }
            Some(Point::new(entry.get("x")?.as_f64()?, entry.get("y")?.as_f64()?))
        })
        .collect()
}

pub fn button(args: &Value) -> MouseButton {
    string(args, "button")
        .and_then(|text| MouseButton::parse(&text))
        .unwrap_or(MouseButton::Left)
}
