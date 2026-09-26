//! 图像类操作：截图、裁剪、取色、差分。
//!
//! 坐标单位统一在这里解析：显式 `--scale`/`--origin` 优先于 sidecar；
//! `units` 未指定时，有锚点就按点坐标解释，没有锚点按图像像素。

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use uitap_core::backend::Backend;
use uitap_core::geom::{Anchor, Point, Rect};
use uitap_core::jsonout::{num, point_json, rect_json, size_json};
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

/// 坐标单位的取值：显式指定优先，未指定时有锚点按点、没有锚点按像素。
///
/// 没有锚点的图（用户给的外部截图）没有「点」可言，此时按像素解释而不是报错，
/// 换算结果与实际传入的坐标一致。
fn units_for(units: Option<Units>, anchor: Option<Anchor>) -> Units {
    match units {
        Some(units) => units,
        None if anchor.is_some() => Units::Point,
        None => Units::Pixel,
    }
}

/// 显式要了点坐标却没有锚点时的说法：给图名、说清为什么、给出两条出路。
fn missing_anchor_error(path: &Path) -> String {
    format!(
        "{} 旁边没有锚点，给不出点坐标（它不是 uitap 截的图）：坐标改按图像像素给，\
         或用 shot 重截一次（shot 会在图旁写 origin 与 scale）",
        path.display()
    )
}

/// 数值在提示里的写法：最多两位小数，去掉尾随零。
fn short(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    let text = format!("{rounded:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// 区域越出这张图时给出能照着改的说法：报越出的是哪几条边、这张图的范围是多少、
/// 当前按哪种坐标解释。空字符串表示没有越界。
fn region_out_of_range(
    region: Rect,
    image: &RgbaImage,
    units: Units,
    anchor: Option<Anchor>,
) -> Option<String> {
    let full = Rect::new(0.0, 0.0, image.width as f64, image.height as f64);
    let limit = match (units, anchor) {
        (Units::Point, Some(anchor)) => anchor.to_point(full),
        _ => full,
    };

    let mut axes: Vec<String> = Vec::new();
    if region.min_x() < limit.min_x() || region.max_x() > limit.max_x() {
        axes.push(format!(
            "x {}..{} 越出 {}..{}",
            short(region.min_x()),
            short(region.max_x()),
            short(limit.min_x()),
            short(limit.max_x())
        ));
    }
    if region.min_y() < limit.min_y() || region.max_y() > limit.max_y() {
        axes.push(format!(
            "y {}..{} 越出 {}..{}",
            short(region.min_y()),
            short(region.max_y()),
            short(limit.min_y()),
            short(limit.max_y())
        ));
    }
    if axes.is_empty() {
        return None;
    }

    let frame = match (units, anchor) {
        (Units::Point, Some(anchor)) => {
            let origin = format!("{},{}", short(anchor.origin.x), short(anchor.origin.y));
            format!(
                "{}x{} 点（origin {} / scale {}）",
                short(full.w),
                short(full.h),
                origin,
                short(anchor.scale)
            )
        }
        _ => format!("{}x{} 像素", image.width, image.height),
    };
    Some(format!(
        "region 超出这张图：{}。这张图是 {}，region 按{}坐标给",
        axes.join("；"),
        frame,
        if units == Units::Point { "点" } else { "像素" }
    ))
}

/// `#RRGGBB` 或 `RRGGBB`。
pub fn parse_color_text(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 3];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// 字符串色值，或 `[r, g, b]`。
pub fn parse_color(value: &Value) -> Option<[u8; 3]> {
    if let Some(array) = value.as_array() {
        if array.len() >= 3 {
            return Some([
                clamp_channel(array[0].as_f64()?),
                clamp_channel(array[1].as_f64()?),
                clamp_channel(array[2].as_f64()?),
            ]);
        }
        return None;
    }
    parse_color_text(value.as_str()?)
}

fn clamp_channel(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}/// 截一次图：按需缩放，写出锚点 sidecar。
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
        return Err(format!("没有这个文件：{}", request.input.display()));
    }

    let output = match &request.output {
        Some(path) => path.clone(),
        None => {
            let stem = request.input.with_extension("");
            std::path::PathBuf::from(format!("{}-crop.png", stem.display()))
        }
    };

    let image = load(&request.input)?;
    let anchor = resolve_anchor(&request.input, &request.anchor_override);

    // `region` 按像素；`regionPoints` 按点，图旁没有锚点时没有「点」可言，按像素解释。
    let (input_region, units) = match (request.region, request.region_points) {
        (Some(region), _) => (Some(region), Units::Pixel),
        (None, Some(points)) if anchor.is_some() => (Some(points), Units::Point),
        (None, Some(points)) => (Some(points), Units::Pixel),
        (None, None) => (None, Units::Pixel),
    };
    let pixel_region = match (input_region, units) {
        (Some(region), Units::Point) => {
            anchor.map(|anchor| store::point_rect_to_pixels(&anchor, region))
        }
        (Some(region), Units::Pixel) => Some(region),
        (None, _) => None,
    };

    if let Some(region) = input_region {
        if let Some(message) = region_out_of_range(region, &image, units, anchor) {
            return Err(message);
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
    if input_region.is_some() {
        map.insert(
            "regionUnits".into(),
            Value::String(units.as_str().into()),
        );
    }

    // 缩放或裁剪后的锚点：原锚点按裁剪偏移与缩放比例调整。
    if let Some(anchor) = anchor {
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
    let point_units = units_for(request.units, anchor) == Units::Point;
    if point_units && anchor.is_none() {
        return Err(missing_anchor_error(&request.path));
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
    let point_units = units_for(request.units, anchor) == Units::Point;
    if point_units && anchor.is_none() {
        return Err(missing_anchor_error(&request.before));
    }

    if let Some(region) = request.region {
        let image = load(&request.before)?;
        let units = if point_units { Units::Point } else { Units::Pixel };
        if let Some(message) = region_out_of_range(region, &image, units, anchor) {
            return Err(message);
        }
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
