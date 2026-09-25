//! 输入操作的互斥编排。
//!
//! 每个输入操作在动手前取一次跨进程租约，做完立刻释放。拿不到时**不阻塞到底**，
//! 而是返回一条可操作的忙碌信息：谁在占用、还要多久、怎么重试。
//!
//! 为什么只锁输入：读操作（截图、取色、比图、读元素树）本身不改变状态，即便与别人的
//! 输入交错，结果也只是「看到对方的动作」，不会把机器搞乱。真正会互相破坏的是两处同时
//! 往同一个界面里送输入。
//!
//! `ui_tap` 是唯一的例外：它必须在整段「点击 → 等稳定 → 比对」期间独占，否则它的验证
//! 结论会被别人的输入污染。因此它自己不调用这里的包装，而是整段持有。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use uitap_core::lease::{Acquire, AcquireRequest, LeaseGuard, LeaseInfo};
use uitap_core::store;

use crate::types::OpResult;

/// 每次操作生成的唯一 token。同一进程内并发调用也能区分开。
static TOKEN_SEQ: AtomicU64 = AtomicU64::new(0);

/// 租约参数。默认开启，因为单 agent 场景下开销是一次文件创建与删除。
#[derive(Clone, Debug)]
pub struct LeaseSettings {
    pub enabled: bool,
    /// 持有时长上限。远大于任何单次操作，仅用于持有者崩溃后的兜底回收。
    pub ttl_ms: u64,
    /// 拿不到时最多等多久。
    pub wait_ms: u64,
}

impl Default for LeaseSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_ms: 30_000,
            // 覆盖一次典型操作（含 ui_tap 的等待与比对），让串行场景直接跑通。
            wait_ms: 5_000,
        }
    }
}

impl LeaseSettings {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    pub fn with_wait(mut self, wait_ms: u64) -> Self {
        self.wait_ms = wait_ms;
        self
    }

    pub fn without_lock(mut self) -> Self {
        self.enabled = false;
        self
    }
}

pub fn lease_path() -> PathBuf {
    store::lease_path()
}

/// 本进程的稳定标识。同一进程的多次操作共用，便于识别「自己挡了自己」。
///
/// 可用 `UITAP_AGENT` 环境变量覆盖，多 agent 场景下建议各自设一个，出错信息里就能直接
/// 看出是谁在占用。
pub fn agent_label() -> String {
    match std::env::var("UITAP_AGENT") {
        Ok(label) if !label.trim().is_empty() => label.trim().to_string(),
        _ => format!("pid:{}", std::process::id()),
    }
}

fn next_token(purpose: &str) -> String {
    let seq = TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{seq}", std::process::id(), purpose)
}

/// 进程存活判定。由平台层提供，用于把「持有者已崩溃」与「持有者仍在忙」区分开。
fn is_alive(pid: i32) -> bool {
    uitap_platform::process_alive(pid)
}

/// 当前有效的租约。给 `ui_lease status` 用。
pub fn current_lease() -> Option<LeaseInfo> {
    uitap_core::lease::current(&lease_path(), &is_alive)
}

/// 强制清除租约。给人工排障用：持有者卡死又没到 TTL 时的逃生口。
pub fn clear_lease() -> bool {
    let path = lease_path();
    std::fs::remove_file(&path).is_ok()
}

/// 忙碌时的错误文本。要够模型自己决定：等多久、还是先做别的。
///
/// 特意把「建议重试间隔」与「持有上限剩余」分开写：前者是行动指引，后者只是对方崩溃时
/// 才会用到的兜底时限。只报后者会让调用方以为要等几十秒而放弃。
fn busy_message(holder: &LeaseInfo, same_agent: bool, purpose: &str) -> String {
    let who = if same_agent {
        format!("同一个 agent 的另一次操作（{}）", holder.agent)
    } else {
        format!("另一个 agent（{}）", holder.agent)
    };
    let what = holder
        .purpose
        .as_deref()
        .map(|p| format!("正在执行 {p}"))
        .unwrap_or_else(|| "正在操作".to_string());

    let ttl_remaining = holder.remaining_ms(uitap_core::lease::now_ms());
    let retry = Acquire::suggested_retry_for(ttl_remaining);

    let mut message = format!(
        "{purpose} 未执行：{who} {what}。建议 {retry}ms 后重试，或加 waitMs 让它自己等。"
    );
    if ttl_remaining > 5_000 {
        message.push_str(&format!(
            "对方最长持有可能还有 {}s（崩溃时的兜底时限）。",
            ttl_remaining / 1000
        ));
    }
    message.push_str("若确认对方已卡死，用 ui_lease(action=\"clear\") 清除。");
    message
}

/// 取租约并执行。拿不到时返回忙碌错误，闭包不会被执行。
pub fn with_lease<T>(
    settings: &LeaseSettings,
    purpose: &str,
    run: impl FnOnce() -> OpResult<T>,
) -> OpResult<T> {
    if !settings.enabled {
        return run();
    }

    let path = lease_path();
    let agent = agent_label();
    let token = next_token(purpose);
    let request = AcquireRequest {
        path: &path,
        agent: &agent,
        token: &token,
        purpose: Some(purpose),
        pid: std::process::id() as i32,
        ttl_ms: settings.ttl_ms,
        wait_ms: settings.wait_ms,
        poll_ms: 50,
    };

    match uitap_core::lease::acquire(&request, &is_alive) {
        Ok(Acquire::Acquired(info)) => {
            let _guard = LeaseGuard::new(path.clone(), info);
            run()
        }
        Ok(Acquire::Busy { holder, same_agent }) => {
            Err(busy_message(&holder, same_agent, purpose))
        }
        Err(error) => Err(format!("取租约失败：{error}")),
    }
}

/// 租约状态与人工控制。
pub fn lease_report(action: &str) -> OpResult<serde_json::Value> {
    use serde_json::{Map, Value};

    let mut map = Map::new();
    map.insert("path".into(), Value::String(lease_path().to_string_lossy().into_owned()));
    map.insert("agent".into(), Value::String(agent_label()));

    match action {
        "status" => {
            match current_lease() {
                Some(info) => {
                    map.insert("held".into(), Value::Bool(true));
                    map.insert("holder".into(), holder_json(&info));
                }
                None => {
                    map.insert("held".into(), Value::Bool(false));
                }
            }
            Ok(Value::Object(map))
        }
        "clear" => {
            let cleared = clear_lease();
            map.insert("cleared".into(), Value::Bool(cleared));
            Ok(Value::Object(map))
        }
        other => Err(format!(
            "未知动作：{other}（可用：status / clear）"
        )),
    }
}

fn holder_json(info: &LeaseInfo) -> serde_json::Value {
    let now = uitap_core::lease::now_ms();
    let mut map = serde_json::Map::new();
    map.insert("agent".into(), serde_json::Value::String(info.agent.clone()));
    map.insert("pid".into(), serde_json::Value::from(info.pid));
    if let Some(purpose) = &info.purpose {
        map.insert("purpose".into(), serde_json::Value::String(purpose.clone()));
    }
    map.insert(
        "remainingMs".into(),
        serde_json::Value::from(info.remaining_ms(now)),
    );
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_settings_run_without_touching_disk() {
        let settings = LeaseSettings::disabled();
        let out = with_lease(&settings, "ui_click", || Ok(1)).unwrap();
        assert_eq!(out, 1);
    }

    #[test]
    fn agent_label_is_stable_within_process() {
        assert_eq!(agent_label(), agent_label());
        assert!(!agent_label().is_empty());
    }

    #[test]
    fn tokens_are_unique_per_call() {
        let a = next_token("ui_click");
        let b = next_token("ui_click");
        assert_ne!(a, b, "并发操作必须能区分，否则自己挡不住自己");
    }

    #[test]
    fn busy_message_tells_the_caller_what_to_do() {
        let now = uitap_core::lease::now_ms();
        let holder = LeaseInfo {
            agent: "pid:999".into(),
            token: "t".into(),
            pid: 999,
            purpose: Some("ui_tap".into()),
            acquired_at_ms: now,
            // TTL 还剩 30 秒：这是兜底上限，不是对方的工作时长。
            expires_at_ms: now + 30_000,
        };

        let other = busy_message(&holder, false, "ui_click");
        assert!(other.contains("pid:999"), "要说明是谁在占用：{other}");
        assert!(other.contains("ui_tap"), "要说明对方在做什么：{other}");
        // 建议值应是轮询量级，而不是 30 秒。
        assert!(other.contains("建议 1000ms 后重试"), "建议应可执行：{other}");
        assert!(other.contains("30s"), "应说明兜底时限：{other}");
        assert!(other.contains("ui_lease"), "要给出出路：{other}");

        let same = busy_message(&holder, true, "ui_click");
        assert!(same.contains("同一个 agent"), "应能识别自己挡自己：{same}");
    }

    #[test]
    fn short_ttl_does_not_add_noise() {
        let now = uitap_core::lease::now_ms();
        let holder = LeaseInfo {
            agent: "a".into(),
            token: "t".into(),
            pid: 1,
            purpose: None,
            acquired_at_ms: now,
            expires_at_ms: now + 800,
        };
        let message = busy_message(&holder, false, "ui_click");
        assert!(!message.contains("兜底时限"), "短 TTL 不该提兜底时限：{message}");
        assert!(message.contains("正在操作"), "没有 purpose 时也要有说法：{message}");
    }

    #[test]
    fn lease_report_rejects_unknown_action() {
        assert!(lease_report("nope").is_err());
    }

    #[test]
    fn lease_report_status_works_without_a_holder() {
        // 状态查询不该因为无人持有而失败。
        let out = lease_report("status").unwrap();
        assert!(out.get("path").is_some());
        assert!(out.get("held").is_some());
    }
}
