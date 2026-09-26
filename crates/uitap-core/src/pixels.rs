//! 位图读写、裁剪缩放与逐像素差分。
//!
//! 差分算法与 Swift 版 `ImageOps.swift` 等价：逐像素三通道绝对差求和，
//! 以单元格聚合后做八邻域连通填充归并变化区域；区域边界取实际变化像素的极值，
//! 整幅 `bounds` 同样取全部变化像素的极值（与区域归并结果无关）。

use std::path::Path;

use image::{DynamicImage, RgbaImage as ImgRgba};

use crate::geom::Rect;

/// RGBA8 位图，行 0 对应图像顶部（与 PNG、屏幕点坐标同向）。
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Self {
        Self { width, height, data }
    }

    pub fn color(&self, x: i64, y: i64) -> Option<(u8, u8, u8, u8)> {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        let index = (y as usize * self.width + x as usize) * 4;
        Some((
            self.data[index],
            self.data[index + 1],
            self.data[index + 2],
            self.data[index + 3],
        ))
    }
}

#[derive(Debug)]
pub enum PixelError {
    Io(String),
    Decode(String),
}

impl std::fmt::Display for PixelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PixelError::Io(m) => write!(f, "{m}"),
            PixelError::Decode(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for PixelError {}

type Result<T> = std::result::Result<T, PixelError>;

pub fn load_rgba(path: &Path) -> Result<RgbaImage> {
    let img = image::open(path).map_err(|e| PixelError::Decode(format!("{}: {e}", path.display())))?;
    let rgba = img.to_rgba8();
    Ok(RgbaImage::new(
        rgba.width() as usize,
        rgba.height() as usize,
        rgba.into_raw(),
    ))
}

pub fn pixel_size(path: &Path) -> Result<(usize, usize)> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| PixelError::Io(format!("{}: {e}", path.display())))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| PixelError::Io(format!("{}: {e}", path.display())))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| PixelError::Decode(format!("{}: {e}", path.display())))?;
    Ok((w as usize, h as usize))
}

pub fn save_png(image: &RgbaImage, path: &Path) -> Result<()> {
    let buffer: ImgRgba = ImgRgba::from_raw(image.width as u32, image.height as u32, image.data.clone())
        .ok_or_else(|| PixelError::Decode("buffer size mismatch".into()))?;
    buffer
        .save(path)
        .map_err(|e| PixelError::Io(format!("{}: {e}", path.display())))
}

/// 从既有图像裁剪指定像素区域，并按最长边缩放写出 PNG。返回最终像素尺寸。
pub fn crop_and_scale(
    source: &RgbaImage,
    region: Option<Rect>,
    max_px: Option<usize>,
    out: &Path,
) -> Result<(usize, usize)> {
    let full = Rect::new(0.0, 0.0, source.width as f64, source.height as f64);
    let crop = match region {
        Some(r) => r.intersect(&full).integral(),
        None => full,
    };
    let x0 = crop.min_x().max(0.0) as u32;
    let y0 = crop.min_y().max(0.0) as u32;
    let w = (crop.w.max(0.0) as u32).min(source.width as u32 - x0.min(source.width as u32));
    let h = (crop.h.max(0.0) as u32).min(source.height as u32 - y0.min(source.height as u32));
    if w == 0 || h == 0 {
        return Err(PixelError::Decode("empty crop region".into()));
    }

    let raw = ImgRgba::from_raw(source.width as u32, source.height as u32, source.data.clone())
        .ok_or_else(|| PixelError::Decode("buffer size mismatch".into()))?;
    let sub = DynamicImage::ImageRgba8(raw).crop_imm(x0, y0, w, h);

    let (tw, th) = match max_px {
        Some(limit) if limit > 0 => {
            let longest = w.max(h) as usize;
            if longest > limit {
                let k = limit as f64 / longest as f64;
                (
                    ((w as f64 * k).round() as usize).max(1),
                    ((h as f64 * k).round() as usize).max(1),
                )
            } else {
                (w as usize, h as usize)
            }
        }
        _ => (w as usize, h as usize),
    };

    let resized = if (tw, th) != (w as usize, h as usize) {
        sub.resize_exact(tw as u32, th as u32, image::imageops::FilterType::Lanczos3)
    } else {
        sub
    };

    let out_rgba = resized.to_rgba8();
    out_rgba
        .save(out)
        .map_err(|e| PixelError::Io(format!("{}: {e}", out.display())))?;
    Ok((out_rgba.width() as usize, out_rgba.height() as usize))
}

/// 单通道色差不超过 `tolerance` 即算命中，与 `pixel --expect` 的判定一致。
pub fn color_within(pixel: (u8, u8, u8, u8), target: [u8; 3], tolerance: f64) -> bool {
    [(pixel.0, target[0]), (pixel.1, target[1]), (pixel.2, target[2])]
        .iter()
        .all(|(actual, want)| (*actual as f64 - *want as f64).abs() <= tolerance)
}

/// 一片命中区域：像素数与包围盒（像素，左上角含、右下角不含）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorCluster {
    pub count: usize,
    pub bounds: Rect,
}

/// 目标色的命中掩码。掩码只覆盖限定区域，坐标是图像内的像素。
#[derive(Clone, Debug, PartialEq)]
pub struct ColorMask {
    /// 掩码左上角在图像内的像素坐标。
    pub x0: usize,
    pub y0: usize,
    pub w: usize,
    pub h: usize,
    /// 行主序，长度 `w * h`。
    pub hits: Vec<bool>,
    /// 与目标色一一对应的命中数。
    pub per_color: Vec<usize>,
}

impl ColorMask {
    pub fn total(&self) -> usize {
        self.per_color.iter().sum()
    }

    /// 全部命中像素的包围盒，无命中时为 None。
    pub fn bounds(&self) -> Option<Rect> {
        let mut min_x = usize::MAX;
        let mut min_y = usize::MAX;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        let mut any = false;
        for y in 0..self.h {
            for x in 0..self.w {
                if !self.hits[y * self.w + x] {
                    continue;
                }
                any = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        if !any {
            return None;
        }
        Some(Rect::new(
            (self.x0 + min_x) as f64,
            (self.y0 + min_y) as f64,
            (max_x - min_x + 1) as f64,
            (max_y - min_y + 1) as f64,
        ))
    }

    /// 八邻域连通聚簇，按像素数降序（同数按上、左优先）。
    /// 第二个返回值表示 `max_clusters` 之外还有聚簇被丢掉。
    pub fn into_clusters(mut self, min_pixels: usize, max_clusters: usize) -> (Vec<ColorCluster>, bool) {
        let mut clusters: Vec<ColorCluster> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();

        for start in 0..self.hits.len() {
            if !self.hits[start] {
                continue;
            }
            self.hits[start] = false;
            stack.push(start);

            let mut count = 0usize;
            let mut min_x = usize::MAX;
            let mut min_y = usize::MAX;
            let mut max_x = 0usize;
            let mut max_y = 0usize;

            while let Some(index) = stack.pop() {
                let x = index % self.w;
                let y = index / self.w;
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);

                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let nx = x as i64 + dx;
                        let ny = y as i64 + dy;
                        if nx < 0 || ny < 0 || nx >= self.w as i64 || ny >= self.h as i64 {
                            continue;
                        }
                        let neighbour = ny as usize * self.w + nx as usize;
                        if self.hits[neighbour] {
                            self.hits[neighbour] = false;
                            stack.push(neighbour);
                        }
                    }
                }
            }

            if count >= min_pixels.max(1) {
                clusters.push(ColorCluster {
                    count,
                    bounds: Rect::new(
                        (self.x0 + min_x) as f64,
                        (self.y0 + min_y) as f64,
                        (max_x - min_x + 1) as f64,
                        (max_y - min_y + 1) as f64,
                    ),
                });
            }
        }

        clusters.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.bounds.min_y().partial_cmp(&b.bounds.min_y()).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.bounds.min_x().partial_cmp(&b.bounds.min_x()).unwrap_or(std::cmp::Ordering::Equal))
        });
        let truncated = clusters.len() > max_clusters;
        clusters.truncate(max_clusters);
        (clusters, truncated)
    }
}

/// 在限定区域内找命中任一目标色的像素。区域按像素，超出图像的部分被裁掉。
pub fn color_mask(
    image: &RgbaImage,
    region: Option<Rect>,
    colors: &[[u8; 3]],
    tolerance: f64,
) -> ColorMask {
    let full = Rect::new(0.0, 0.0, image.width as f64, image.height as f64);
    let scan = match region {
        Some(region) => region.intersect(&full).integral(),
        None => full,
    };
    let x0 = (scan.min_x().max(0.0) as usize).min(image.width);
    let y0 = (scan.min_y().max(0.0) as usize).min(image.height);
    let x1 = (scan.max_x().max(0.0) as usize).min(image.width);
    let y1 = (scan.max_y().max(0.0) as usize).min(image.height);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);

    let mut hits = vec![false; w * h];
    let mut per_color = vec![0usize; colors.len()];
    for y in 0..h {
        for x in 0..w {
            let Some(pixel) = image.color((x0 + x) as i64, (y0 + y) as i64) else {
                continue;
            };
            for (slot, target) in colors.iter().enumerate() {
                if color_within(pixel, *target, tolerance) {
                    hits[y * w + x] = true;
                    per_color[slot] += 1;
                    break;
                }
            }
        }
    }

    ColorMask {
        x0,
        y0,
        w,
        h,
        hits,
        per_color,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DiffOptions {
    /// 单像素视为变化的色差阈值（三通道绝对差之和）。
    pub threshold: i32,
    /// 连通填充的单元格边长。
    pub cell: usize,
    /// 区域最小像素数，低于此值的区域被丢弃。
    pub min_pixels: usize,
    /// 返回区域数上限。
    pub max_regions: usize,
    /// 限定比对范围，像素坐标。
    pub region: Option<Rect>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            threshold: 24,
            cell: 16,
            min_pixels: 12,
            max_regions: 6,
            region: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffRegion {
    pub rect: Rect,
    pub pixels: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffResult {
    pub width: usize,
    pub height: usize,
    pub changed_pixels: usize,
    pub sampled_pixels: usize,
    /// 像素坐标，全部变化像素的包围盒。
    pub bounds: Option<Rect>,
    pub regions: Vec<DiffRegion>,
}

impl DiffResult {
    pub fn changed(&self) -> bool {
        self.changed_pixels > 0
    }

    pub fn ratio(&self) -> f64 {
        if self.sampled_pixels == 0 {
            0.0
        } else {
            self.changed_pixels as f64 / self.sampled_pixels as f64
        }
    }
}

#[derive(Debug)]
pub struct SizeMismatch {
    pub a: (usize, usize),
    pub b: (usize, usize),
}

impl std::fmt::Display for SizeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "size mismatch: {}x{} vs {}x{}",
            self.a.0, self.a.1, self.b.0, self.b.1
        )
    }
}

impl std::error::Error for SizeMismatch {}

pub fn diff(a: &RgbaImage, b: &RgbaImage, opts: &DiffOptions) -> std::result::Result<DiffResult, SizeMismatch> {
    if a.width != b.width || a.height != b.height {
        return Err(SizeMismatch {
            a: (a.width, a.height),
            b: (b.width, b.height),
        });
    }

    let width = a.width;
    let height = a.height;
    let full = Rect::new(0.0, 0.0, width as f64, height as f64);
    let scan = match opts.region {
        Some(r) => r.intersect(&full).integral(),
        None => full,
    };

    let x0 = (scan.min_x().max(0.0) as usize).min(width);
    let y0 = (scan.min_y().max(0.0) as usize).min(height);
    let x1 = (scan.max_x().max(0.0) as usize).min(width);
    let y1 = (scan.max_y().max(0.0) as usize).min(height);

    if x1 <= x0 || y1 <= y0 {
        return Ok(DiffResult {
            width,
            height,
            changed_pixels: 0,
            sampled_pixels: 0,
            bounds: None,
            regions: Vec::new(),
        });
    }

    let cell = opts.cell.max(1);
    let cols = (x1 - x0).div_ceil(cell);
    let rows = (y1 - y0).div_ceil(cell);
    let cell_count_total = cols * rows;

    let mut hit = vec![false; cell_count_total];
    let mut cell_min_x = vec![usize::MAX; cell_count_total];
    let mut cell_min_y = vec![usize::MAX; cell_count_total];
    let mut cell_max_x = vec![0usize; cell_count_total];
    let mut cell_max_y = vec![0usize; cell_count_total];
    let mut cell_pixels = vec![0usize; cell_count_total];

    let mut changed_pixels = 0usize;
    let mut min_x = usize::MAX;
    let mut min_y = usize::MAX;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    for y in y0..y1 {
        let row = y * width * 4;
        let gy = (y - y0) / cell;
        for x in x0..x1 {
            let i = row + x * 4;
            let delta = (a.data[i] as i32 - b.data[i] as i32).abs()
                + (a.data[i + 1] as i32 - b.data[i + 1] as i32).abs()
                + (a.data[i + 2] as i32 - b.data[i + 2] as i32).abs();
            if delta <= opts.threshold {
                continue;
            }
            changed_pixels += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);

            let ci = gy * cols + (x - x0) / cell;
            hit[ci] = true;
            cell_pixels[ci] += 1;
            cell_min_x[ci] = cell_min_x[ci].min(x);
            cell_min_y[ci] = cell_min_y[ci].min(y);
            cell_max_x[ci] = cell_max_x[ci].max(x);
            cell_max_y[ci] = cell_max_y[ci].max(y);
        }
    }

    let mut regions: Vec<DiffRegion> = Vec::new();
    if changed_pixels > 0 {
        let mut visited = vec![false; cell_count_total];
        for start in 0..cell_count_total {
            if !hit[start] || visited[start] {
                continue;
            }
            visited[start] = true;
            let mut stack = vec![start];
            let mut r_min_x = usize::MAX;
            let mut r_min_y = usize::MAX;
            let mut r_max_x = 0usize;
            let mut r_max_y = 0usize;
            let mut sum = 0usize;

            while let Some(idx) = stack.pop() {
                if cell_min_x[idx] != usize::MAX {
                    r_min_x = r_min_x.min(cell_min_x[idx]);
                    r_min_y = r_min_y.min(cell_min_y[idx]);
                    r_max_x = r_max_x.max(cell_max_x[idx]);
                    r_max_y = r_max_y.max(cell_max_y[idx]);
                }
                sum += cell_pixels[idx];

                let cy = idx / cols;
                let cx = idx % cols;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let nxc = cx as i64 + dx;
                        let nyc = cy as i64 + dy;
                        if nxc < 0 || nyc < 0 || nxc >= cols as i64 || nyc >= rows as i64 {
                            continue;
                        }
                        let ni = nyc as usize * cols + nxc as usize;
                        if hit[ni] && !visited[ni] {
                            visited[ni] = true;
                            stack.push(ni);
                        }
                    }
                }
            }

            if r_min_x == usize::MAX {
                continue;
            }
            regions.push(DiffRegion {
                rect: Rect::new(
                    r_min_x as f64,
                    r_min_y as f64,
                    (r_max_x - r_min_x + 1) as f64,
                    (r_max_y - r_min_y + 1) as f64,
                ),
                pixels: sum,
            });
        }

        regions.sort_by(|l, r| r.pixels.cmp(&l.pixels));
        if opts.min_pixels > 0 {
            regions.retain(|r| r.pixels >= opts.min_pixels);
        }
        if opts.max_regions > 0 && regions.len() > opts.max_regions {
            regions.truncate(opts.max_regions);
        }
    }

    let bounds = if changed_pixels > 0 {
        Some(Rect::new(
            min_x as f64,
            min_y as f64,
            (max_x - min_x + 1) as f64,
            (max_y - min_y + 1) as f64,
        ))
    } else {
        None
    };

    Ok(DiffResult {
        width,
        height,
        changed_pixels,
        sampled_pixels: (x1 - x0) * (y1 - y0),
        bounds,
        regions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 Swift 版验证过的合成用例一致：
    /// 200x100 白底，红块 30x20 @(40,60)，蓝块 10x10 @(150,10)。
    fn synthetic_pair() -> (RgbaImage, RgbaImage) {
        let (w, h) = (200usize, 100usize);
        let mut a = RgbaImage::new(w, h, vec![255; w * h * 4]);
        let mut b = a.clone();
        let put = |img: &mut RgbaImage, x0: usize, y0: usize, bw: usize, bh: usize, rgb: [u8; 3]| {
            for y in y0..y0 + bh {
                for x in x0..x0 + bw {
                    let i = (y * w + x) * 4;
                    img.data[i] = rgb[0];
                    img.data[i + 1] = rgb[1];
                    img.data[i + 2] = rgb[2];
                }
            }
        };
        put(&mut b, 40, 60, 30, 20, [255, 0, 0]);
        put(&mut b, 150, 10, 10, 10, [0, 0, 255]);
        (a, b)
    }

    #[test]
    fn detects_both_regions_exactly() {
        let (a, b) = synthetic_pair();
        let result = diff(&a, &b, &DiffOptions::default()).unwrap();

        assert!(result.changed());
        assert_eq!(result.changed_pixels, 700);
        assert_eq!(result.sampled_pixels, 200 * 100);
        assert_eq!(result.bounds, Some(Rect::new(40.0, 10.0, 120.0, 70.0)));

        assert_eq!(result.regions.len(), 2);
        // 按像素数降序：600 的红块在前，100 的蓝块在后。
        assert_eq!(result.regions[0].rect, Rect::new(40.0, 60.0, 30.0, 20.0));
        assert_eq!(result.regions[0].pixels, 600);
        assert_eq!(result.regions[1].rect, Rect::new(150.0, 10.0, 10.0, 10.0));
        assert_eq!(result.regions[1].pixels, 100);
    }

    #[test]
    fn identical_images_report_no_change() {
        let (a, _) = synthetic_pair();
        let result = diff(&a, &a, &DiffOptions::default()).unwrap();
        assert!(!result.changed());
        assert_eq!(result.changed_pixels, 0);
        assert!(result.bounds.is_none());
        assert!(result.regions.is_empty());
    }

    #[test]
    fn region_limit_restricts_scan() {
        let (a, b) = synthetic_pair();
        // 只比对左半幅，蓝块在 x=150 被排除。
        let opts = DiffOptions {
            region: Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
            ..DiffOptions::default()
        };
        let result = diff(&a, &b, &opts).unwrap();
        assert_eq!(result.changed_pixels, 600);
        assert_eq!(result.sampled_pixels, 100 * 100);
        assert_eq!(result.regions.len(), 1);
        assert_eq!(result.regions[0].rect, Rect::new(40.0, 60.0, 30.0, 20.0));
    }

    #[test]
    fn min_pixels_filters_small_regions() {
        let (a, b) = synthetic_pair();
        let opts = DiffOptions {
            min_pixels: 200,
            ..DiffOptions::default()
        };
        let result = diff(&a, &b, &opts).unwrap();
        // 区域被过滤，但 bounds 仍覆盖全部变化像素。
        assert_eq!(result.regions.len(), 1);
        assert_eq!(result.changed_pixels, 700);
        assert_eq!(result.bounds, Some(Rect::new(40.0, 10.0, 120.0, 70.0)));
    }

    #[test]
    fn size_mismatch_is_rejected() {
        let a = RgbaImage::new(2, 2, vec![0; 16]);
        let b = RgbaImage::new(3, 2, vec![0; 24]);
        assert!(diff(&a, &b, &DiffOptions::default()).is_err());
    }

    /// 100x100 白底：(10,10) 起 5x4 红块、(60,70) 起 2x2 红块、(80,5) 单点红。
    fn color_fixture() -> RgbaImage {
        let mut image = RgbaImage::new(100, 100, vec![255; 100 * 100 * 4]);
        let mut put = |x0: usize, y0: usize, w: usize, h: usize, rgb: [u8; 3]| {
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    let index = (y * 100 + x) * 4;
                    image.data[index] = rgb[0];
                    image.data[index + 1] = rgb[1];
                    image.data[index + 2] = rgb[2];
                }
            }
        };
        put(10, 10, 5, 4, [255, 0, 0]);
        put(60, 70, 2, 2, [255, 0, 0]);
        put(80, 5, 1, 1, [255, 0, 0]);
        image
    }

    #[test]
    fn color_mask_counts_and_bounds() {
        let image = color_fixture();
        let mask = color_mask(&image, None, &[[255, 0, 0]], 12.0);
        assert_eq!(mask.total(), 20 + 4 + 1);
        assert_eq!(mask.bounds(), Some(Rect::new(10.0, 5.0, 71.0, 67.0)));
    }

    #[test]
    fn color_mask_honours_tolerance_and_region() {
        let image = color_fixture();
        // 容差 0 时白底不算命中红。
        let strict = color_mask(&image, None, &[[255, 0, 0]], 0.0);
        assert_eq!(strict.total(), 25);
        // 区域限定到左半幅，红块只剩 20 像素。
        let limited = color_mask(&image, Some(Rect::new(0.0, 0.0, 50.0, 50.0)), &[[255, 0, 0]], 0.0);
        assert_eq!(limited.total(), 20);
        assert_eq!(limited.bounds(), Some(Rect::new(10.0, 10.0, 5.0, 4.0)));
    }

    #[test]
    fn color_mask_splits_clusters_by_connectivity() {
        let image = color_fixture();
        let mask = color_mask(&image, None, &[[255, 0, 0]], 0.0);
        let (clusters, truncated) = mask.into_clusters(1, 8);
        assert!(!truncated);
        assert_eq!(clusters.len(), 3);
        assert_eq!(clusters[0].count, 20);
        assert_eq!(clusters[0].bounds, Rect::new(10.0, 10.0, 5.0, 4.0));
        assert_eq!(clusters[1].count, 4);
        assert_eq!(clusters[2].count, 1);
    }

    #[test]
    fn min_pixels_and_max_clusters_bound_the_result() {
        let image = color_fixture();
        let mask = color_mask(&image, None, &[[255, 0, 0]], 0.0);
        let (clusters, truncated) = mask.into_clusters(2, 8);
        // 单点被 minPixels 滤掉。
        assert_eq!(clusters.len(), 2);
        assert!(!truncated);

        let mask = color_mask(&image, None, &[[255, 0, 0]], 0.0);
        let (clusters, truncated) = mask.into_clusters(1, 2);
        assert_eq!(clusters.len(), 2);
        assert!(truncated, "还有聚簇被丢掉时应报 truncated");
    }

    #[test]
    fn per_color_counts_are_separate() {
        let image = color_fixture();
        let mask = color_mask(&image, None, &[[255, 0, 0], [255, 255, 255]], 0.0);
        assert_eq!(mask.per_color, vec![25, 9975]);
        assert_eq!(mask.total(), 10000);
    }
}
