//! 等待画面稳定：反复截图，直到连续若干帧的变化比例低于阈值，或超时。

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Map, Value};

use uitap_core::backend::{Backend, CaptureTarget};
use uitap_core::jsonout::ratio;
use uitap_core::pixels::{self, DiffOptions};
use uitap_core::store;

use crate::image::load;
use crate::types::{OpResult, WaitParams};

/// 稳定性判定用的单像素色差阈值。固定值，与可调的变化比例阈值是两回事。
const STABILITY_PIXEL_DELTA: i32 = 24;

pub struct WaitOutcome {
    pub stable: bool,
    pub samples: usize,
    pub elapsed_ms: u64,
    pub last_ratio: f64,
    /// 最后一帧的路径，供 `tap` 直接用于差分。
    pub last_path: PathBuf,
}

pub fn wait_stable(
    backend: &dyn Backend,
    target: &CaptureTarget,
    params: &WaitParams,
    seed: Option<&Path>,
) -> OpResult<WaitOutcome> {
    let interval = params.interval_ms.max(40);
    let timeout = params.timeout_ms.max(200);
    let need_stable = params.stable_samples.max(1);

    let mut previous: PathBuf = match seed {
        Some(path) => path.to_path_buf(),
        None => {
            let path = store::new_shot_path("wait");
            backend
                .capture(target, &path)
                .map_err(|e| e.to_string())?;
            path
        }
    };

    let started = Instant::now();
    let mut stable_count = 0usize;
    let mut attempts = 0usize;
    let mut last_ratio = 1.0f64;
    let mut use_a = true;

    while started.elapsed().as_millis() < timeout as u128 {
        let next = store::frame_path(if use_a { 2 } else { 1 });
        backend
            .capture(target, &next)
            .map_err(|e| e.to_string())?;
        attempts += 1;

        if let (Ok(lhs), Ok(rhs)) = (pixels::load_rgba(&previous), pixels::load_rgba(&next)) {
            let options = DiffOptions {
                threshold: STABILITY_PIXEL_DELTA,
                min_pixels: 0,
                max_regions: 0,
                ..DiffOptions::default()
            };
            if let Ok(diff) = pixels::diff(&lhs, &rhs, &options) {
                last_ratio = diff.ratio();
                stable_count = if last_ratio <= params.ratio_threshold {
                    stable_count + 1
                } else {
                    0
                };
                previous = next;
                use_a = !use_a;
            }
        }

        if stable_count >= need_stable {
            return Ok(WaitOutcome {
                stable: true,
                samples: attempts,
                elapsed_ms: started.elapsed().as_millis() as u64,
                last_ratio,
                last_path: previous,
            });
        }

        std::thread::sleep(std::time::Duration::from_millis(interval));
    }

    Ok(WaitOutcome {
        stable: false,
        samples: attempts,
        elapsed_ms: started.elapsed().as_millis() as u64,
        last_ratio,
        last_path: previous,
    })
}

pub fn wait_stable_json(
    backend: &dyn Backend,
    target: &CaptureTarget,
    params: &WaitParams,
) -> OpResult<Value> {
    let outcome = wait_stable(backend, target, params, None)?;

    let mut map = Map::new();
    map.insert("stable".into(), Value::Bool(outcome.stable));
    map.insert("samples".into(), Value::from(outcome.samples));
    map.insert("elapsedMs".into(), Value::from(outcome.elapsed_ms));
    map.insert("lastRatio".into(), ratio(outcome.last_ratio));
    Ok(Value::Object(map))
}

/// 供调用方在差分后清理中间帧。
pub fn clear_frames() {
    let _ = std::fs::remove_file(store::frame_path(1));
    let _ = std::fs::remove_file(store::frame_path(2));
}

/// 读取一张图并保证可解码，失败时带上路径。
pub fn read_frame(path: &Path) -> OpResult<pixels::RgbaImage> {
    load(path)
}
