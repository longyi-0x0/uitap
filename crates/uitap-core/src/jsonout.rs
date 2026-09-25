//! 与 Swift 版逐字对应的 JSON 数值约定。
//!
//! - `num`：整数值不带小数点，其余保留一位。
//! - `ratio`：固定保留四位，去掉尾随零。
//!
//! 两者都先落到十进制字符串再回读，避免 f64 二进制噪声（如 0.035000000000000003）。
//! serde_json 默认用 BTreeMap，键自然有序，与 Swift 的 `.sortedKeys` 一致。

use serde_json::{Map, Number, Value};

use crate::geom::{Point, Rect};

fn decimal(text: String) -> Value {
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => value,
        Err(_) => Value::Number(Number::from(0)),
    }
}

fn trim_trailing_zeros(text: String) -> String {
    if !text.contains('.') {
        return text;
    }
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 整数化：整数值输出整数，其余保留一位小数。
pub fn num(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Number(Number::from(0));
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 && rounded.abs() < 1e9 {
        return Value::Number(Number::from(rounded as i64));
    }
    decimal(trim_trailing_zeros(format!("{rounded:.1}")))
}

/// 比例：固定四位，去尾随零。
pub fn ratio(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Number(Number::from(0));
    }
    decimal(trim_trailing_zeros(format!("{value:.4}")))
}

pub fn point_json(p: Point) -> Value {
    let mut map = Map::new();
    map.insert("x".into(), num(p.x));
    map.insert("y".into(), num(p.y));
    Value::Object(map)
}

pub fn rect_json(r: Rect) -> Value {
    let mut map = Map::new();
    map.insert("x".into(), num(r.x));
    map.insert("y".into(), num(r.y));
    map.insert("w".into(), num(r.w));
    map.insert("h".into(), num(r.h));
    Value::Object(map)
}

pub fn size_json(w: usize, h: usize) -> Value {
    let mut map = Map::new();
    map.insert("w".into(), Value::Number(Number::from(w)));
    map.insert("h".into(), Value::Number(Number::from(h)));
    Value::Object(map)
}

/// 输出单行紧凑 JSON。
pub fn to_line(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| r#"{"error":"encode failed"}"#.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_integerizes() {
        assert_eq!(to_line(&num(0.0)), "0");
        assert_eq!(to_line(&num(1080.0)), "1080");
        assert_eq!(to_line(&num(1959.4)), "1959.4");
        assert_eq!(to_line(&num(1959.0)), "1959");
    }

    #[test]
    fn ratio_keeps_four_places_without_noise() {
        assert_eq!(to_line(&ratio(0.035)), "0.035");
        assert_eq!(to_line(&ratio(0.0001)), "0.0001");
        assert_eq!(to_line(&ratio(0.0)), "0");
        assert_eq!(to_line(&ratio(0.11642)), "0.1164");
    }

    #[test]
    fn keys_are_sorted() {
        let v = rect_json(Rect::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(to_line(&v), r#"{"h":4,"w":3,"x":1,"y":2}"#);
    }
}
