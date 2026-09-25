//! 命令行 → 操作层入参的映射。这一层只做解析与打印。

use std::path::PathBuf;

use uitap_core::backend::{CaptureTarget, MouseButton};
use uitap_core::geom::Point;
use uitap_core::backend::{ElementAction, TreeLimits};
use uitap_ops::{
    ActivateRequest, AnchorOverride, AppTarget, CropRequest, DiffRequest, ElementQuery,
    PixelRequest, ScrollRequest, ShotRequest, TapRequest, Units, WaitParams, WindowQuery,
};

use crate::args::Args;

/// `--window` / `--region` / `--display` 三选一，都不给则主显示器。
pub fn target(a: &Args) -> CaptureTarget {
    if let Some(window) = a.str("window").and_then(|v| v.parse::<u64>().ok()) {
        return CaptureTarget::Window(window);
    }
    if let Some(region) = a.rect("region") {
        return CaptureTarget::Region(region);
    }
    if a.has("display") {
        return CaptureTarget::Screen {
            display_index: Some(a.usize("display", 0)),
        };
    }
    CaptureTarget::Screen {
        display_index: None,
    }
}

pub fn anchor_override(a: &Args) -> AnchorOverride {
    AnchorOverride {
        scale: a.str("scale").and_then(|v| v.parse::<f64>().ok()),
        origin: a.point("origin"),
    }
}

pub fn units(a: &Args) -> Option<Units> {
    a.str("units").and_then(Units::parse)
}

pub fn window_query(a: &Args) -> WindowQuery {
    WindowQuery {
        all: a.flag("all"),
        app: a.str("app").map(str::to_string),
        title: a.str("title").map(str::to_string),
        layer: a.has("layer").then(|| a.int("layer", 0) as i32),
        min_width: a.double("minWidth", 0.0),
        min_height: a.double("minHeight", 0.0),
        front_only: a.flag("frontOnly"),
        limit: a.has("limit").then(|| a.int("limit", 0).max(0) as usize),
    }
}

pub fn shot_request(a: &Args, tag: &str) -> ShotRequest {
    ShotRequest {
        target: target(a),
        path: a.str("path").map(PathBuf::from),
        max_px: a.str("maxPx").and_then(|v| v.parse::<usize>().ok()),
        tag: tag.to_string(),
    }
}

pub fn crop_request(a: &Args, input: PathBuf) -> CropRequest {
    CropRequest {
        input,
        output: a.str("out").map(PathBuf::from),
        region: a.rect("region"),
        region_points: a.rect("regionPoints"),
        max_px: a.str("maxPx").and_then(|v| v.parse::<usize>().ok()),
        anchor_override: anchor_override(a),
    }
}

pub fn pixel_request(a: &Args, path: PathBuf) -> PixelRequest {
    PixelRequest {
        path,
        points: a.points("at"),
        units: units(a),
        anchor_override: anchor_override(a),
    }
}

pub fn diff_request(a: &Args) -> DiffRequest {
    let mut request = DiffRequest::new(
        PathBuf::from(a.str("before").unwrap_or_default()),
        PathBuf::from(a.str("after").unwrap_or_default()),
    );
    request.threshold = a.str("threshold").and_then(|v| v.parse().ok());
    request.cell = a.str("cell").and_then(|v| v.parse().ok());
    request.min_pixels = a.str("minPixels").and_then(|v| v.parse().ok());
    request.max_regions = a.str("maxRegions").and_then(|v| v.parse().ok());
    request.region = a.rect("region");
    request.units = units(a);
    request.anchor_override = anchor_override(a);
    request
}

/// `wait-stable` 与 `tap` 共用；两者的 `--threshold` 语义不同，调用方负责区分。
pub fn wait_params(a: &Args) -> WaitParams {
    WaitParams {
        interval_ms: a.int("interval", 120).max(40) as u64,
        timeout_ms: a.int("timeout", 4000).max(200) as u64,
        ratio_threshold: a.double("threshold", 0.0006),
        stable_samples: a.int("stableSamples", 2).max(1) as usize,
    }
}

/// `tap` 的 `--threshold` 沿用单像素色差语义，因此稳定比例保持默认。
pub fn tap_wait_params(a: &Args) -> WaitParams {
    WaitParams {
        interval_ms: a.int("interval", 120).max(40) as u64,
        timeout_ms: a.int("timeout", 4000).max(200) as u64,
        ratio_threshold: WaitParams::default().ratio_threshold,
        stable_samples: a.int("stableSamples", 2).max(1) as usize,
    }
}

pub fn tap_request(a: &Args, at: Point) -> TapRequest {
    TapRequest {
        at,
        target: target(a),
        button: button(a),
        count: a.int("count", 1).max(1) as u32,
        settle_ms: a.int("settle", 120).max(0) as u64,
        wait: tap_wait_params(a),
        units: units(a),
        keep: a.flag("keep"),
    }
}

pub fn button(a: &Args) -> MouseButton {
    MouseButton::parse(a.str("button").unwrap_or("left")).unwrap_or(MouseButton::Left)
}

pub fn scroll_request(a: &Args) -> ScrollRequest {
    ScrollRequest {
        at: a.point("at"),
        dx: a.int("dx", 0) as i32,
        dy: a.int("dy", 0) as i32,
    }
}

pub fn activate_request(a: &Args) -> ActivateRequest {
    ActivateRequest {
        app: a.str("app").map(str::to_string),
        pid: a.str("pid").and_then(|v| v.parse().ok()),
        window: a.str("window").and_then(|v| v.parse().ok()),
    }
}

// ---------------------------------------------------------------- 元素

/// 元素操作的目标：`--app` / `--pid` / `--window` 三选一，与观测类命令一致。
pub fn app_target(a: &Args) -> AppTarget {
    AppTarget::from_parts(
        a.str("app").map(str::to_string),
        a.str("pid").and_then(|v| v.parse().ok()),
        a.str("window").and_then(|v| v.parse().ok()),
    )
    .unwrap_or_else(|| fail_target())
}

fn fail_target() -> ! {
    crate::out::fail("--app、--pid 或 --window 至少给一个，用来确定目标应用")
}

pub fn tree_limits(a: &Args) -> TreeLimits {
    let defaults = TreeLimits::default();
    TreeLimits {
        max_depth: a.int("depth", defaults.max_depth as i64).max(0) as usize,
        max_nodes: a.int("maxNodes", defaults.max_nodes as i64).max(1) as usize,
    }
}

pub fn element_query(a: &Args) -> ElementQuery {
    ElementQuery {
        role: a.str("role").map(str::to_string),
        subrole: a.str("subrole").map(str::to_string),
        title: a.str("title").map(str::to_string),
        value: a.str("value").map(str::to_string),
        identifier: a.str("identifier").map(str::to_string),
        enabled_only: a.flag("enabled"),
    }
}

/// `--window` 限定元素树范围；给 `--pid`/`--app` 时不限定。
pub fn element_window(a: &Args) -> Option<u64> {
    if a.str("pid").is_some() || a.str("app").is_some() {
        return None;
    }
    a.str("window").and_then(|v| v.parse().ok())
}

pub fn element_action(a: &Args) -> ElementAction {
    let text = a.str("action").unwrap_or("press");
    ElementAction::parse(text).unwrap_or_else(|| {
        crate::out::fail(format!(
            "未知动作：{text}（可用：press / showMenu / increment / decrement / confirm / cancel / pick）"
        ))
    })
}
