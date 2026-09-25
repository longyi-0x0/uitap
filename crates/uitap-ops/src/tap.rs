//! 组合动作：点击 → 等画面稳定 → 与点击前比对。
//!
//! 一次调用替代「截图 + 等稳定 + 差分」三次往返，返回变化的点坐标。

use std::fs;

use serde_json::{Map, Value};

use uitap_core::backend::Backend;
use uitap_core::geom::Anchor;
use uitap_core::jsonout::point_json;
use uitap_core::pixels::{self, DiffOptions};
use uitap_core::store;

use crate::image::{load, shot};
use crate::json::diff_json;
use crate::types::{OpResult, ShotRequest, TapRequest, Units};
use crate::wait::{clear_frames, wait_stable};

pub fn tap(backend: &dyn Backend, request: &TapRequest) -> OpResult<Value> {
    let before_request = ShotRequest {
        target: request.target.clone(),
        path: None,
        max_px: None,
        tag: "tap-before".to_string(),
    };
    let before = shot(backend, &before_request)?;

    backend
        .click(request.at, request.button, request.count.max(1))
        .map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_millis(request.settle_ms));

    let outcome = wait_stable(
        backend,
        &request.target,
        &request.wait,
        Some(before.path()),
    )?;

    let base = load(before.path())?;
    let final_frame = load(&outcome.last_path)?;
    let result = pixels::diff(&base, &final_frame, &DiffOptions::default())
        .map_err(|e| e.to_string())?;

    let anchor = Anchor::new(before.capture.origin, before.capture.scale());
    let as_pixel = request.units == Some(Units::Pixel);

    let mut map = match diff_json(&result, Some(anchor), !as_pixel) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("ok".into(), Value::Bool(true));
    map.insert("at".into(), point_json(request.at));
    map.insert("stable".into(), Value::Bool(outcome.stable));
    map.insert("elapsedMs".into(), Value::from(outcome.elapsed_ms));

    if request.keep {
        let after_path = std::path::PathBuf::from(
            before
                .path()
                .to_string_lossy()
                .replace("tap-before", "tap-after"),
        );
        let _ = fs::copy(&outcome.last_path, &after_path);
        map.insert(
            "before".into(),
            Value::String(before.path().to_string_lossy().into_owned()),
        );
        map.insert(
            "after".into(),
            Value::String(after_path.to_string_lossy().into_owned()),
        );
    } else {
        let _ = fs::remove_file(before.path());
        let _ = fs::remove_file(store::sidecar_path(before.path()));
    }
    clear_frames();

    Ok(Value::Object(map))
}
