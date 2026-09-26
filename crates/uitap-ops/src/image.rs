//! 图像类操作：截图、裁剪、取色、按色找像素、差分。
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
    AnchorOverride, CropRequest, DiffRequest, FindPixelsRequest, OpResult, PixelRequest,
    ShotOutcome, ShotRequest, Units,
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

/// 按颜色找像素：命中数、每种色的命中数、包围盒与连通聚簇。
/// 输出坐标与输入 `region` 同一坐标系：有锚点给点坐标，没有给像素。
pub fn find_pixels(request: &FindPixelsRequest) -> OpResult<Value> {
    if request.colors.is_empty() {
        return Err("至少给一个目标色，写成 #RRGGBB 或 [r, g, b]".into());
    }

    let image = load(&request.path)?;
    let anchor = resolve_anchor(&request.path, &request.anchor_override);
    let units = units_for(request.units, anchor);
    if units == Units::Point && anchor.is_none() {
        return Err(missing_anchor_error(&request.path));
    }

    if let Some(region) = request.region {
        if let Some(message) = region_out_of_range(region, &image, units, anchor) {
            return Err(message);
        }
    }

    let region_pixels = match (units, request.region, anchor) {
        (Units::Point, Some(region), Some(anchor)) => {
            Some(store::point_rect_to_pixels(&anchor, region))
        }
        (_, region, _) => region,
    };

    let mask = pixels::color_mask(&image, region_pixels, &request.colors, request.tolerance);
    let convert = |rect: Rect| -> Rect {
        match (units, anchor) {
            (Units::Point, Some(anchor)) => anchor.to_point(rect),
            _ => rect,
        }
    };

    let mut map = Map::new();
    map.insert("units".into(), Value::String(units.as_str().into()));
    map.insert("size".into(), size_json(image.width, image.height));
    if let Some(region) = request.region {
        map.insert("region".into(), rect_json(region));
    }
    map.insert("count".into(), Value::from(mask.total()));
    map.insert(
        "perColor".into(),
        Value::Array(
            request
                .colors
                .iter()
                .enumerate()
                .map(|(index, color)| {
                    let mut item = Map::new();
                    item.insert(
                        "color".into(),
                        Value::String(format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])),
                    );
                    item.insert(
                        "count".into(),
                        Value::from(mask.per_color.get(index).copied().unwrap_or(0)),
                    );
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    if let Some(bounds) = mask.bounds() {
        map.insert("bbox".into(), rect_json(convert(bounds)));
    }

    let (clusters, truncated) = mask.into_clusters(request.min_pixels, request.max_clusters);
    map.insert(
        "clusters".into(),
        Value::Array(
            clusters
                .iter()
                .map(|cluster| {
                    let mut item = match rect_json(convert(cluster.bounds)) {
                        Value::Object(map) => map,
                        _ => Map::new(),
                    };
                    item.insert("pixels".into(), Value::from(cluster.count));
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    if truncated {
        map.insert("clustersTruncated".into(), Value::Bool(true));
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 100x80 的图：(10,10) 起 4x3 红块、(60,50) 起 2x2 红块，其余白底。
    fn fixture_image() -> RgbaImage {
        let mut image = RgbaImage::new(100, 80, vec![255; 100 * 80 * 4]);
        for (x0, y0, w, h) in [(10usize, 10usize, 4usize, 3usize), (60, 50, 2, 2)] {
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    let index = (y * 100 + x) * 4;
                    image.data[index] = 255;
                    image.data[index + 1] = 0;
                    image.data[index + 2] = 0;
                }
            }
        }
        image
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("uitap-ops-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("建临时目录");
            Self(path)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 写一张图；给了 `anchor` 就连同 sidecar 一起写，与 shot 的产物同形。
    fn write_image(path: &Path, image: &RgbaImage, anchor: Option<(Point, f64)>) {
        pixels::save_png(image, path).expect("写图");
        if let Some((origin, scale)) = anchor {
            let sidecar = serde_json::json!({
                "path": path.to_string_lossy(),
                "origin": { "x": origin.x, "y": origin.y },
                "scale": scale,
            });
            fs::write(store::sidecar_path(path), sidecar.to_string()).expect("写 sidecar");
        }
    }

    fn crop_request(input: &Path, region_points: Rect) -> CropRequest {
        CropRequest {
            input: input.to_path_buf(),
            output: None,
            region: None,
            region_points: Some(region_points),
            max_px: None,
            anchor_override: AnchorOverride::default(),
        }
    }

    #[test]
    fn region_out_of_range_names_the_edge_and_the_frame() {
        let dir = TempDir::new("range-point");
        let path = dir.file("shot.png");
        // 2048x1080 点的屏按 1500/2048 缩放后的图，与真实会话里那次失败同形。
        let image = RgbaImage::new(1500, 791, vec![255; 1500 * 791 * 4]);
        write_image(
            &path,
            &image,
            Some((Point::new(1920.0, 0.0), 1500.0 / 2048.0)),
        );

        let error = crop(&crop_request(&path, Rect::new(1900.0, 480.0, 600.0, 220.0)))
            .expect_err("越界应报错");
        assert!(error.contains("x 1900..2500 越出 1920..3968"), "{error}");
        assert!(error.contains("region 按点坐标给"), "{error}");

        // 起点落在图左侧：越出的是左边界，不是宽高。
        let error = crop(&crop_request(&path, Rect::new(1180.0, 480.0, 260.0, 220.0)))
            .expect_err("越界应报错");
        assert!(error.contains("x 1180..1440 越出 1920..3968"), "{error}");
    }

    #[test]
    fn region_out_of_range_in_pixels_reports_pixel_frame() {
        let dir = TempDir::new("range-pixel");
        let path = dir.file("plain.png");
        write_image(&path, &fixture_image(), None);

        let request = CropRequest {
            region: Some(Rect::new(0.0, 0.0, 200.0, 40.0)),
            ..crop_request(&path, Rect::new(0.0, 0.0, 0.0, 0.0))
        };
        let error = crop(&request).expect_err("越界应报错");
        assert!(error.contains("x 0..200 越出 0..100"), "{error}");
        assert!(error.contains("100x80 像素"), "{error}");
        assert!(error.contains("region 按像素坐标给"), "{error}");
    }

    #[test]
    fn region_points_fall_back_to_pixels_without_anchor() {
        let dir = TempDir::new("no-anchor-crop");
        let path = dir.file("pasted.png");
        write_image(&path, &fixture_image(), None);

        // 同一份 regionPoints：有锚点时按点，没有锚点时按像素，不再报错。
        let out = crop(&crop_request(&path, Rect::new(10.0, 10.0, 4.0, 3.0))).expect("应能裁");
        assert_eq!(out.get("regionUnits").and_then(Value::as_str), Some("pixel"));
        assert!(out.get("origin").is_none(), "没有锚点就不该报 origin");
    }

    #[test]
    fn pixel_units_follow_the_anchor() {
        let dir = TempDir::new("pixel-units");
        let plain = dir.file("plain.png");
        let shot = dir.file("shot.png");
        write_image(&plain, &fixture_image(), None);
        write_image(
            &shot,
            &fixture_image(),
            Some((Point::new(100.0, 50.0), 1.0)),
        );

        let request = PixelRequest {
            path: plain.clone(),
            points: vec![Point::new(10.0, 10.0)],
            units: None,
            anchor_override: AnchorOverride::default(),
        };
        let out = pixel(&request).expect("无锚点按像素取色");
        assert_eq!(out.get("units").and_then(Value::as_str), Some("pixel"));
        assert_eq!(
            out.pointer("/points/0/hex").and_then(Value::as_str),
            Some("#FF0000")
        );

        // 同一份点坐标，对有锚点的图按点解释：(110, 60) 才是那块红。
        let request = PixelRequest {
            path: shot,
            points: vec![Point::new(110.0, 60.0)],
            units: None,
            anchor_override: AnchorOverride::default(),
        };
        let out = pixel(&request).expect("有锚点按点取色");
        assert_eq!(out.get("units").and_then(Value::as_str), Some("point"));
        assert_eq!(
            out.pointer("/points/0/hex").and_then(Value::as_str),
            Some("#FF0000")
        );
    }

    #[test]
    fn explicit_point_units_without_anchor_says_what_to_do() {
        let dir = TempDir::new("explicit-point");
        let path = dir.file("pasted.png");
        write_image(&path, &fixture_image(), None);

        let request = PixelRequest {
            path,
            points: vec![Point::new(10.0, 10.0)],
            units: Some(Units::Point),
            anchor_override: AnchorOverride::default(),
        };
        let error = pixel(&request).expect_err("显式点坐标但没有锚点应报错");
        assert!(error.contains("旁边没有锚点"), "{error}");
        assert!(error.contains("改按图像像素给"), "{error}");
    }

    #[test]
    fn find_pixels_reports_counts_bounds_and_clusters() {
        let dir = TempDir::new("find-pixels");
        let path = dir.file("plain.png");
        write_image(&path, &fixture_image(), None);

        let request = FindPixelsRequest {
            path,
            region: None,
            units: None,
            colors: vec![[255, 0, 0]],
            tolerance: 12.0,
            min_pixels: 1,
            max_clusters: 8,
            anchor_override: AnchorOverride::default(),
        };
        let out = find_pixels(&request).expect("找色");
        assert_eq!(out.get("units").and_then(Value::as_str), Some("pixel"));
        assert_eq!(out.get("count").and_then(Value::as_u64), Some(16));
        assert_eq!(out.pointer("/bbox/x").and_then(Value::as_u64), Some(10));
        assert_eq!(out.pointer("/bbox/y").and_then(Value::as_u64), Some(10));
        assert_eq!(out.pointer("/bbox/w").and_then(Value::as_u64), Some(52));
        assert_eq!(out.pointer("/bbox/h").and_then(Value::as_u64), Some(42));
        assert_eq!(
            out.pointer("/clusters/0/pixels").and_then(Value::as_u64),
            Some(12)
        );
        assert_eq!(
            out.pointer("/clusters/1/pixels").and_then(Value::as_u64),
            Some(4)
        );
    }

    #[test]
    fn find_pixels_converts_to_points_with_anchor() {
        let dir = TempDir::new("find-pixels-point");
        let path = dir.file("shot.png");
        write_image(&path, &fixture_image(), Some((Point::new(200.0, 100.0), 2.0)));

        let request = FindPixelsRequest {
            path,
            region: None,
            units: None,
            colors: vec![[255, 0, 0]],
            tolerance: 12.0,
            min_pixels: 1,
            max_clusters: 8,
            anchor_override: AnchorOverride::default(),
        };
        let out = find_pixels(&request).expect("找色");
        assert_eq!(out.get("units").and_then(Value::as_str), Some("point"));
        // 像素 (10,10) 在原点 (200,100)、scale 2 下是点 (205, 105)。
        assert_eq!(out.pointer("/clusters/0/x").and_then(Value::as_f64), Some(205.0));
        assert_eq!(out.pointer("/clusters/0/y").and_then(Value::as_f64), Some(105.0));
    }

    #[test]
    fn find_pixels_requires_a_color() {
        let dir = TempDir::new("find-pixels-empty");
        let path = dir.file("plain.png");
        write_image(&path, &fixture_image(), None);
        let request = FindPixelsRequest {
            path,
            region: None,
            units: None,
            colors: Vec::new(),
            tolerance: 12.0,
            min_pixels: 1,
            max_clusters: 8,
            anchor_override: AnchorOverride::default(),
        };
        assert!(find_pixels(&request).is_err());
    }

    #[test]
    fn colors_parse_from_text_and_triples() {
        assert_eq!(parse_color_text("#2F6BFF"), Some([0x2F, 0x6B, 0xFF]));
        assert_eq!(parse_color_text("2f6bff"), Some([0x2F, 0x6B, 0xFF]));
        assert_eq!(parse_color_text("#2F6B"), None);
        assert_eq!(
            parse_color(&serde_json::json!([47, 107, 255])),
            Some([0x2F, 0x6B, 0xFF])
        );
        assert_eq!(
            parse_color(&serde_json::json!("#2F6BFF")),
            Some([0x2F, 0x6B, 0xFF])
        );
    }
}
