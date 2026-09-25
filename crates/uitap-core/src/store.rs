//! 截图落盘、锚点 sidecar 与目录裁剪。
//!
//! 锚点随图落盘，后续取色与差分不必再传 scale 与 origin。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::backend::RawCapture;
use crate::geom::{Anchor, Point, Rect};

pub use crate::pixels::PixelError;

/// 临时截图目录。macOS 与 Linux 沿用 `/tmp/uitap`，与既有文档一致。
pub fn shot_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp/uitap")
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("uitap")
    }
}

pub fn ensure_shot_dir() -> PathBuf {
    let dir = shot_dir();
    let _ = fs::create_dir_all(&dir);
    dir
}

/// 等待画面稳定时使用的帧文件，裁剪时受保护。
pub const FRAME_A: &str = "uitap-frame-a.png";
pub const FRAME_B: &str = "uitap-frame-b.png";

pub fn frame_path(slot: u8) -> PathBuf {
    ensure_shot_dir().join(if slot == 1 { FRAME_B } else { FRAME_A })
}

pub fn new_shot_path(tag: &str) -> PathBuf {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ensure_shot_dir().join(format!("{tag}-{ms}.png"))
}

pub fn sidecar_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_os_string();
    name.push(".json");
    PathBuf::from(name)
}

pub fn write_sidecar(image: &Path, capture: &RawCapture) -> std::io::Result<()> {
    let mut map = Map::new();
    map.insert(
        "path".into(),
        Value::String(image.to_string_lossy().into_owned()),
    );
    map.insert(
        "origin".into(),
        Value::Object({
            let mut o = Map::new();
            o.insert("x".into(), number(capture.origin.x));
            o.insert("y".into(), number(capture.origin.y));
            o
        }),
    );
    map.insert("scale".into(), number(capture.scale()));
    map.insert(
        "size".into(),
        Value::Object({
            let mut o = Map::new();
            o.insert("w".into(), Value::from(capture.pixel_width));
            o.insert("h".into(), Value::from(capture.pixel_height));
            o
        }),
    );
    map.insert("mode".into(), Value::String(capture.mode.as_str().into()));
    if let Some(id) = capture.window_id {
        map.insert("window".into(), Value::from(id));
    }

    let text = serde_json::to_string(&Value::Object(map)).unwrap_or_default();
    fs::write(sidecar_path(image), text)
}

fn number(value: f64) -> Value {
    match serde_json::Number::from_f64(value) {
        Some(n) => Value::Number(n),
        None => Value::from(0),
    }
}

/// 读取锚点。显式传入的 scale/origin 优先于 sidecar。
pub fn resolve_anchor(image: &Path, explicit: Option<Anchor>) -> Option<Anchor> {
    if let Some(anchor) = explicit {
        return Some(anchor);
    }
    let text = fs::read_to_string(sidecar_path(image)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let origin = value.get("origin")?;
    let x = origin.get("x")?.as_f64()?;
    let y = origin.get("y")?.as_f64()?;
    let scale = value.get("scale")?.as_f64()?;
    if scale <= 0.0 {
        return None;
    }
    Some(Anchor::new(Point::new(x, y), scale))
}

/// 目录内文件过多时裁掉较旧的一半，避免临时截图无限堆积。
pub fn prune(keep: usize) {
    let dir = shot_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };

    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name != FRAME_A && name != FRAME_B
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let time = meta.created().or_else(|_| meta.modified()).unwrap_or(UNIX_EPOCH);
            Some((entry.path(), time))
        })
        .collect();

    if files.len() <= keep {
        return;
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let remove = files.len() - keep / 2;
    for (path, _) in files.into_iter().take(remove) {
        let _ = fs::remove_file(path);
    }
}

/// 把点空间的矩形换算成像素空间矩形，供 `diff --region` 这类点坐标入参使用。
pub fn point_rect_to_pixels(anchor: &Anchor, region: Rect) -> Rect {
    let top_left = anchor.to_pixel(Point::new(region.min_x(), region.min_y()));
    let bottom_right = anchor.to_pixel(Point::new(region.max_x(), region.max_y()));
    Rect::new(
        top_left.x,
        top_left.y,
        bottom_right.x - top_left.x,
        bottom_right.y - top_left.y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::CaptureMode;

    fn sample_capture() -> RawCapture {
        RawCapture {
            path: PathBuf::from("/tmp/uitap/sample.png"),
            pixel_width: 200,
            pixel_height: 100,
            origin: Point::new(84.0, 30.0),
            point_width: 100.0,
            point_height: 50.0,
            mode: CaptureMode::Window,
            window_id: Some(7),
        }
    }

    #[test]
    fn scale_derives_from_final_image_size() {
        assert_eq!(sample_capture().scale(), 2.0);
    }

    #[test]
    fn sidecar_roundtrip() {
        let dir = std::env::temp_dir().join("uitap-sidecar-test");
        let _ = fs::create_dir_all(&dir);
        let image = dir.join("t.png");
        let capture = sample_capture();
        write_sidecar(&image, &capture).unwrap();

        let anchor = resolve_anchor(&image, None).unwrap();
        assert_eq!(anchor.origin, Point::new(84.0, 30.0));
        assert_eq!(anchor.scale, 2.0);
        let _ = fs::remove_file(sidecar_path(&image));
    }

    #[test]
    fn explicit_anchor_beats_sidecar() {
        let dir = std::env::temp_dir().join("uitap-sidecar-test");
        let _ = fs::create_dir_all(&dir);
        let image = dir.join("t2.png");
        write_sidecar(&image, &sample_capture()).unwrap();

        let explicit = Anchor::new(Point::new(0.0, 0.0), 1.0);
        assert_eq!(resolve_anchor(&image, Some(explicit)), Some(explicit));
        let _ = fs::remove_file(sidecar_path(&image));
    }

    #[test]
    fn point_rect_maps_to_full_pixel_area() {
        // Retina 窗口：左上角全局点 (84,30)，100x50 点 -> 200x100 像素。
        let anchor = Anchor::new(Point::new(84.0, 30.0), 2.0);
        let got = point_rect_to_pixels(&anchor, Rect::new(84.0, 30.0, 100.0, 50.0));
        assert_eq!(got, Rect::new(0.0, 0.0, 200.0, 100.0));
    }
}
