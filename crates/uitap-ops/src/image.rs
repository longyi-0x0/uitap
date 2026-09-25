//! 图像类操作：截图、裁剪、取色、差分。
//!
//! 坐标单位统一在这里解析：显式 `--scale`/`--origin` 优先于 sidecar；
//! `units` 未指定时，有锚点就按点坐标解释。

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use uitap_core::backend::Backend;
use uitap_core::geom::{Anchor, Point};
use uitap_core::jsonout::{num, point_json, size_json};
use uitap_core::pixels::{self, RgbaImage};
use uitap_core::store;

use crate::json::diff_json;
use crate::types::{
    AnchorOverride, CropRequest, DiffRequest, OpResult, PixelRequest, ShotOutcome, ShotRequest,
    Units,
};

pub fn load(path: &Path) -> OpResult<RgbaImage> {
    pixels::load_rgba(path).map_err(|e| format!("cannot read image: {e}"))
}

/// 解析锚点：显式覆盖优先，其次 sidecar。
pub fn resolve_anchor(path: &Path, overrides: &AnchorOverride) -> Option<Anchor> {
    store::resolve_anchor(path, overrides.resolve())
}

fn use_point_units(units: Option<Units>, has_anchor: bool) -> bool {
    match units {
        Some(units) => units == Units::Point,
        None => has_anchor,
    }
}

/// 截一次图：按需缩放，写出锚点 sidecar。
pub fn shot(backend: &dyn Backend, request: &ShotRequest) -> OpResult<ShotOutcome> {
    let path = match &request.path {
        Some(path) => path.clone(),
        None => store::new_shot_path(&request.tag),
    };

    let capture = match request.max_px {
        Some(limit) if limit > 0 => {
            // 先截到暂存文件再缩放写出最终图，保证锚点基于最终像素尺寸。
            let staging = std::path::PathBuf::from(format!("{}.raw.png", path.display()));
            let raw = backend
                .capture(&request.target, &staging)
                .map_err(|e| e.to_string())?;
            let image = load(&staging)?;
            let scaled = pixels::crop_and_scale(&image, None, Some(limit), &path)
                .map_err(|e| format!("resize failed: {e}"))?;
            let _ = fs::remove_file(&staging);
            uitap_core::backend::RawCapture {
                path: path.clone(),
                pixel_width: scaled.0,
                pixel_height: scaled.1,
                ..raw
            }
        }
        _ => backend
            .capture(&request.target, &path)
            .map_err(|e| e.to_string())?,
    };

    let _ = store::write_sidecar(&capture.path, &capture);
    Ok(ShotOutcome { capture })
}

pub fn crop(request: &CropRequest) -> OpResult<Value> {
    if !request.input.exists() {
        return Err(format!("no such file: {}", request.input.display()));
    }

    let output = match &request.output {
        Some(path) => path.clone(),
        None => {
            let stem = request.input.with_extension("");
            std::path::PathBuf::from(format!("{}-crop.png", stem.display()))
        }
    };

    // `region` 按像素，`regionPoints` 按点，后者需要锚点。
    let mut pixel_region = request.region;
    if let Some(region_points) = request.region_points {
        let anchor = resolve_anchor(&request.input, &request.anchor_override)
            .ok_or("--regionPoints needs a sidecar or --scale/--origin")?;
        pixel_region = Some(store::point_rect_to_pixels(&anchor, region_points));
    }

    let image = load(&request.input)?;
    if let Some(region) = pixel_region {
        if region.min_x() < 0.0
            || region.min_y() < 0.0
            || region.max_x() > image.width as f64
            || region.max_y() > image.height as f64
        {
            return Err(format!(
                "region {}x{} exceeds image {}x{}",
                region.w, region.h, image.width, image.height
            ));
        }
    }

    let (width, height) =
        pixels::crop_and_scale(&image, pixel_region, request.max_px, &output)
            .map_err(|e| format!("crop failed: {e}"))?;

    let mut map = Map::new();
    map.insert(
        "path".into(),
        Value::String(output.to_string_lossy().into_owned()),
    );
    map.insert("size".into(), size_json(width, height));

    // 缩放或裁剪后的锚点：原锚点按裁剪偏移与缩放比例调整。
    if let Some(anchor) = resolve_anchor(&request.input, &request.anchor_override) {
        let crop_width = match pixel_region {
            Some(region) => region.w.max(1.0),
            None => width as f64,
        };
        let (min_x, min_y) = match pixel_region {
            Some(region) => (region.min_x(), region.min_y()),
            None => (0.0, 0.0),
        };
        let scale = anchor.effective_scale();
        map.insert(
            "origin".into(),
            point_json(Point::new(
                anchor.origin.x + min_x / scale,
                anchor.origin.y + min_y / scale,
            )),
        );
        map.insert("scale".into(), num(width as f64 / crop_width * anchor.scale));
    }

    Ok(Value::Object(map))
}

pub fn pixel(request: &PixelRequest) -> OpResult<Value> {
    let image = load(&request.path)?;
    if request.points.is_empty() {
        return Err("--at x,y is required".into());
    }

    let anchor = resolve_anchor(&request.path, &request.anchor_override);
    let point_units = use_point_units(request.units, anchor.is_some());
    if point_units && anchor.is_none() {
        return Err("--units point needs a sidecar or --scale/--origin".into());
    }

    let results: Vec<Value> = request
        .points
        .iter()
        .map(|input| {
            let pixel = match (point_units, anchor) {
                (true, Some(anchor)) => anchor.to_pixel(*input),
                _ => *input,
            };
            let x = pixel.x.round() as i64;
            let y = pixel.y.round() as i64;

            let mut map = Map::new();
            map.insert("x".into(), num(input.x));
            map.insert("y".into(), num(input.y));

            match image.color(x, y) {
                Some((r, g, b, _)) => {
                    map.insert("hex".into(), Value::String(format!("#{r:02X}{g:02X}{b:02X}")));
                    map.insert(
                        "rgb".into(),
                        Value::Array(vec![Value::from(r), Value::from(g), Value::from(b)]),
                    );
                    if point_units {
                        map.insert("px".into(), Value::Array(vec![Value::from(x), Value::from(y)]));
                    }
                }
                None => {
                    map.insert("error".into(), Value::String("out of bounds".into()));
                    map.insert("size".into(), size_json(image.width, image.height));
                }
            }
            Value::Object(map)
        })
        .collect();

    let mut map = Map::new();
    map.insert(
        "units".into(),
        Value::String(if point_units { "point" } else { "pixel" }.into()),
    );
    map.insert("points".into(), Value::Array(results));
    Ok(Value::Object(map))
}

pub fn diff(request: &DiffRequest) -> OpResult<Value> {
    let before = load(&request.before)?;
    let after = load(&request.after)?;

    let anchor = resolve_anchor(&request.before, &request.anchor_override);
    let point_units = use_point_units(request.units, anchor.is_some());
    if point_units && anchor.is_none() {
        return Err("--units point needs a sidecar or --scale/--origin".into());
    }

    // `--region` 与 `--units` 同一坐标系：点坐标时先折算成像素再限定比对范围。
    let region_pixels = match (point_units, request.region, anchor) {
        (true, Some(region), Some(anchor)) => Some(store::point_rect_to_pixels(&anchor, region)),
        (_, region, _) => region,
    };

    let result = pixels::diff(&before, &after, &request.options(region_pixels))
        .map_err(|e| e.to_string())?;
    Ok(diff_json(&result, anchor, point_units))
}
