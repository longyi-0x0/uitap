//! 操作层的入参与坐标单位。
//!
//! CLI 与 MCP 都只负责把自己的调用形式映射成这里的结构体，产出 JSON 的逻辑只有一份。

use std::path::PathBuf;

use uitap_core::backend::{CaptureTarget, MouseButton};
use uitap_core::geom::{Anchor, Point, Rect};
use uitap_core::pixels::DiffOptions;

pub type OpResult<T> = Result<T, String>;

/// 坐标单位。`None` 表示由锚点是否存在自动决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    Point,
    Pixel,
}

impl Units {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "point" => Some(Units::Point),
            "pixel" => Some(Units::Pixel),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Units::Point => "point",
            Units::Pixel => "pixel",
        }
    }
}

/// 显式传入的锚点，优先于图像旁的 sidecar。
#[derive(Clone, Copy, Debug, Default)]
pub struct AnchorOverride {
    pub scale: Option<f64>,
    pub origin: Option<Point>,
}

impl AnchorOverride {
    pub fn resolve(&self) -> Option<Anchor> {
        match (self.scale, self.origin) {
            (Some(scale), Some(origin)) if scale > 0.0 => Some(Anchor::new(origin, scale)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowQuery {
    pub all: bool,
    pub app: Option<String>,
    pub title: Option<String>,
    pub layer: Option<i32>,
    pub min_width: f64,
    pub min_height: f64,
    pub front_only: bool,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct ShotRequest {
    pub target: CaptureTarget,
    /// 留空则用 `/tmp/uitap/` 下的时间戳文件名。
    pub path: Option<PathBuf>,
    pub max_px: Option<usize>,
    pub tag: String,
}

impl ShotRequest {
    pub fn new(target: CaptureTarget) -> Self {
        Self {
            target,
            path: None,
            max_px: None,
            tag: "shot".to_string(),
        }
    }
}

/// 一次截图的产物：锚点信息加上已按最终尺寸写盘的路径。
#[derive(Clone, Debug)]
pub struct ShotOutcome {
    pub capture: uitap_core::backend::RawCapture,
}

impl ShotOutcome {
    pub fn path(&self) -> &std::path::Path {
        &self.capture.path
    }

    pub fn json(&self) -> serde_json::Value {
        crate::json::shot_json(&self.capture)
    }
}

#[derive(Clone, Debug)]
pub struct CropRequest {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    /// 像素坐标区域。
    pub region: Option<Rect>,
    /// 点坐标区域，需要锚点。
    pub region_points: Option<Rect>,
    pub max_px: Option<usize>,
    pub anchor_override: AnchorOverride,
}

#[derive(Clone, Debug)]
pub struct PixelRequest {
    pub path: PathBuf,
    pub points: Vec<Point>,
    pub units: Option<Units>,
    pub anchor_override: AnchorOverride,
}

#[derive(Clone, Debug)]
pub struct DiffRequest {
    pub before: PathBuf,
    pub after: PathBuf,
    pub threshold: Option<i32>,
    pub cell: Option<usize>,
    pub min_pixels: Option<usize>,
    pub max_regions: Option<usize>,
    pub region: Option<Rect>,
    pub units: Option<Units>,
    pub anchor_override: AnchorOverride,
}

impl DiffRequest {
    pub fn new(before: PathBuf, after: PathBuf) -> Self {
        Self {
            before,
            after,
            threshold: None,
            cell: None,
            min_pixels: None,
            max_regions: None,
            region: None,
            units: None,
            anchor_override: AnchorOverride::default(),
        }
    }

    /// 沿用与 Swift 版一致的全部默认值。
    pub fn options(&self, region_pixels: Option<Rect>) -> DiffOptions {
        DiffOptions {
            threshold: self.threshold.unwrap_or(24),
            cell: self.cell.unwrap_or(16),
            min_pixels: self.min_pixels.unwrap_or(12),
            max_regions: self.max_regions.unwrap_or(6),
            region: region_pixels,
        }
    }
}

/// 等待画面稳定的参数，`tap` 与 `wait-stable` 共用。
#[derive(Clone, Copy, Debug)]
pub struct WaitParams {
    pub interval_ms: u64,
    pub timeout_ms: u64,
    /// 判定静止的变化比例上限。
    pub ratio_threshold: f64,
    pub stable_samples: usize,
}

impl Default for WaitParams {
    fn default() -> Self {
        Self {
            interval_ms: 120,
            timeout_ms: 4000,
            ratio_threshold: 0.0006,
            stable_samples: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TapRequest {
    pub at: Point,
    pub target: CaptureTarget,
    pub button: MouseButton,
    pub count: u32,
    pub settle_ms: u64,
    pub wait: WaitParams,
    pub units: Option<Units>,
    pub keep: bool,
}

#[derive(Clone, Debug)]
pub struct ActivateRequest {
    pub app: Option<String>,
    pub pid: Option<i32>,
    pub window: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ScrollRequest {
    pub at: Option<Point>,
    pub dx: i32,
    pub dy: i32,
}
