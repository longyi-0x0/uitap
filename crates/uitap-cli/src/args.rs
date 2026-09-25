//! 命令行解析：`--key value`、`--key=value`、裸开关 `--key`；重复的 key 累积成数组。
//! 以 `-` 开头的数值不会被当作 key，`--region -100,20,100,100` 可用。

use uitap_core::geom::{Point, Rect};

pub struct Args {
    values: Vec<(String, String)>,
}

impl Args {
    pub fn new(raw: &[String]) -> Self {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut index = 0usize;

        while index < raw.len() {
            let token = &raw[index];
            if !token.starts_with("--") {
                index += 1;
                continue;
            }
            let body = &token[2..];
            if let Some(eq) = body.find('=') {
                values.push((body[..eq].to_string(), body[eq + 1..].to_string()));
            } else if index + 1 < raw.len() && !raw[index + 1].starts_with("--") {
                values.push((body.to_string(), raw[index + 1].clone()));
                index += 1;
            } else {
                values.push((body.to_string(), "true".to_string()));
            }
            index += 1;
        }

        Self { values }
    }

    pub fn all(&self, key: &str) -> Vec<&str> {
        self.values
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn has(&self, key: &str) -> bool {
        self.values.iter().any(|(k, _)| k == key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn flag(&self, key: &str) -> bool {
        match self.str(key) {
            Some(v) => v != "false" && v != "0",
            None => false,
        }
    }

    pub fn int(&self, key: &str, default: i64) -> i64 {
        self.str(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn double(&self, key: &str, default: f64) -> f64 {
        self.str(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn usize(&self, key: &str, default: usize) -> usize {
        self.str(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    /// 接受 `12,34`、`12 34`、`12x34` 三种写法。
    fn numbers(raw: &str) -> Vec<f64> {
        raw.split([',', ' ', 'x', 'X'])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect()
    }

    pub fn point(&self, key: &str) -> Option<Point> {
        let n = Self::numbers(self.str(key)?);
        if n.len() < 2 {
            return None;
        }
        Some(Point::new(n[0], n[1]))
    }

    pub fn rect(&self, key: &str) -> Option<Rect> {
        let n = Self::numbers(self.str(key)?);
        if n.len() < 4 {
            return None;
        }
        Some(Rect::new(n[0], n[1], n[2], n[3]))
    }

    pub fn points(&self, key: &str) -> Vec<Point> {
        self.all(key)
            .into_iter()
            .filter_map(|raw| {
                let n = Self::numbers(raw);
                if n.len() < 2 {
                    return None;
                }
                Some(Point::new(n[0], n[1]))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        Args::new(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_flags_and_repeats() {
        let a = args(&["--at", "10,20", "--at", "30,40", "--all"]);
        assert_eq!(a.points("at"), vec![Point::new(10.0, 20.0), Point::new(30.0, 40.0)]);
        assert!(a.flag("all"));
        assert!(!a.flag("missing"));
    }

    #[test]
    fn negative_numbers_are_values_not_keys() {
        let a = args(&["--region", "-100,20,50,50"]);
        assert_eq!(
            a.rect("region"),
            Some(Rect::new(-100.0, 20.0, 50.0, 50.0))
        );
    }

    #[test]
    fn equals_form_is_supported() {
        let a = args(&["--path=/tmp/x.png", "--maxPx=800"]);
        assert_eq!(a.str("path"), Some("/tmp/x.png"));
        assert_eq!(a.usize("maxPx", 0), 800);
    }
}
