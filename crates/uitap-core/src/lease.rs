//! 跨进程租约。同一台机器上多个 agent 驱动同一套输入时的互斥。
//!
//! 用文件做仲裁：`create_new` 是原子的，谁建成谁持有。持有者写进自己的标识与到期时间，
//! 因此等待方能回答「谁在占用、还要多久」，而不是干等。
//!
//! 三条不变量：
//!
//! 1. **不会死锁**。租约带到期时间；持有者进程消失或超时，等待方可以接管。
//! 2. **不会误删**。释放前比对 token，过期的租约被别人接管后，原持有者的释放是空操作。
//! 3. **接管是排他的**。过期文件先 `rename` 到唯一名字再删，`rename` 只可能有一个成功，
//!    因此两个等待方不会同时认为「我已接管」。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 租约内容。落盘为 JSON，供其他进程读取。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseInfo {
    /// 稳定的持有方标识，同一进程的多次操作共用，如 `pid:1234`。
    pub agent: String,
    /// 单次持有的唯一标识，释放时比对用。
    pub token: String,
    pub pid: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub acquired_at_ms: u64,
    pub expires_at_ms: u64,
}

impl LeaseInfo {
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at_ms
    }

    /// 剩余时间，已过期则为 0。
    pub fn remaining_ms(&self, now: u64) -> u64 {
        self.expires_at_ms.saturating_sub(now)
    }
}

#[derive(Debug)]
pub enum Acquire {
    Acquired(LeaseInfo),
    /// 别人持有。附上对方信息，以及两个不同用途的时间：
    /// 建议的轮询间隔，与持有上限的剩余量。
    Busy {
        holder: LeaseInfo,
        same_agent: bool,
    },
}

impl Acquire {
    /// 建议等待多久再重试。
    ///
    /// 租约的 TTL 是持有者崩溃时的兜底上限（默认 30s），并不是对方的实际剩余工作时间——
    /// 一次 `ui_click` 只需几十毫秒。直接把它当建议等待时间会误导调用方放弃，
    /// 所以这里给一个轮询量级的建议值。
    pub fn suggested_retry_ms(holder: &LeaseInfo) -> u64 {
        Self::suggested_retry_for(holder.remaining_ms(now_ms()))
    }

    /// 建议重试间隔。给出剩余 TTL 是为了在它很短时不要白等。
    pub fn suggested_retry_for(ttl_remaining_ms: u64) -> u64 {
        const POLL_HINT_MS: u64 = 1_000;
        ttl_remaining_ms.min(POLL_HINT_MS).max(1)
    }
}

/// 一次获取的参数。
#[derive(Clone, Debug)]
pub struct AcquireRequest<'a> {
    pub path: &'a Path,
    pub agent: &'a str,
    pub token: &'a str,
    pub purpose: Option<&'a str>,
    pub pid: i32,
    /// 持有时长上限，防止持有者崩溃后长期占位。
    pub ttl_ms: u64,
    /// 拿不到时最多等多久。
    pub wait_ms: u64,
    /// 等待时的轮询间隔。
    pub poll_ms: u64,
}

/// 读取当前租约。文件不存在、损坏或已过期都返回 `None`。
pub fn peek(path: &Path) -> Option<LeaseInfo> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 当前有效的租约。过期或持有进程已消失时视为无主。
pub fn current(path: &Path, is_alive: &dyn Fn(i32) -> bool) -> Option<LeaseInfo> {
    let info = peek(path)?;
    if info.is_expired(now_ms()) || !is_alive(info.pid) {
        None
    } else {
        Some(info)
    }
}

/// 获取租约。拿不到且等待预算用尽时返回 `Busy`，绝不无限阻塞。
pub fn acquire(request: &AcquireRequest, is_alive: &dyn Fn(i32) -> bool) -> std::io::Result<Acquire> {
    if let Some(parent) = request.path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let deadline = now_ms() + request.wait_ms;
    let poll = request.poll_ms.max(20);

    loop {
        let now = now_ms();
        let info = LeaseInfo {
            agent: request.agent.to_string(),
            token: request.token.to_string(),
            pid: request.pid,
            purpose: request.purpose.map(str::to_string),
            acquired_at_ms: now,
            expires_at_ms: now + request.ttl_ms.max(1_000),
        };

        match try_create(request.path, &info) {
            Ok(()) => return Ok(Acquire::Acquired(info)),
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error),
            Err(_) => {}
        }

        // 已被占用。判断是等、是接管，还是立刻报忙碌。
        //
        // 注意 `peek` 返回 `None` 有两种情况：文件不存在（几乎不可能，`create_new` 刚失败过），
        // 以及文件损坏。两者都按「无主」处理并尝试接管，否则会退化成空转。
        let holder = peek(request.path);
        match holder {
            Some(holder) if !holder.is_expired(now) && is_alive(holder.pid) => {
                if now >= deadline {
                    return Ok(Acquire::Busy {
                        same_agent: holder.agent == request.agent,
                        holder,
                    });
                }
            }
            _ => {
                // 无主、过期或持有者已消失：尝试接管。
                // rename 只可能有一个进程成功，因此接管是排他的。
                let graveyard = with_suffix(
                    request.path,
                    &format!("stale-{}-{}", request.pid, request.token),
                );
                if fs::rename(request.path, &graveyard).is_ok() {
                    let _ = fs::remove_file(&graveyard);
                }
                continue;
            }
        }

        let remaining = deadline.saturating_sub(now);
        sleep(Duration::from_millis(poll.min(remaining.max(1))));
    }
}

fn try_create(path: &Path, info: &LeaseInfo) -> std::io::Result<()> {
    // create_new 是原子断言：文件已存在时返回 AlreadyExists。
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let text = serde_json::to_string(info).unwrap_or_default();
    file.write_all(text.as_bytes())?;
    file.flush()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lease".to_string());
    name.push('.');
    name.push_str(suffix);
    path.with_file_name(name)
}

/// 释放租约。只在 token 匹配时删除，因此不会误删别人接管的租约。
pub fn release(path: &Path, token: &str) -> bool {
    match peek(path) {
        Some(info) if info.token == token => fs::remove_file(path).is_ok(),
        _ => false,
    }
}

/// 续期。只在 token 匹配时生效。
pub fn refresh(path: &Path, token: &str, ttl_ms: u64) -> bool {
    let Some(mut info) = peek(path) else {
        return false;
    };
    if info.token != token {
        return false;
    }
    info.expires_at_ms = now_ms() + ttl_ms.max(1_000);
    let Ok(text) = serde_json::to_string(&info) else {
        return false;
    };
    match File::create(path) {
        Ok(mut file) => file.write_all(text.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// 持有期内自动释放。持有者崩溃时不会执行，此时靠到期时间兜底。
#[derive(Debug)]
pub struct LeaseGuard {
    path: PathBuf,
    info: LeaseInfo,
}

impl LeaseGuard {
    pub fn new(path: PathBuf, info: LeaseInfo) -> Self {
        Self { path, info }
    }

    pub fn info(&self) -> &LeaseInfo {
        &self.info
    }

    pub fn refresh(&mut self, ttl_ms: u64) -> bool {
        if refresh(&self.path, &self.info.token, ttl_ms) {
            self.info.expires_at_ms = now_ms() + ttl_ms.max(1_000);
            true
        } else {
            false
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        release(&self.path, &self.info.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "uitap-lease-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn always_alive(_pid: i32) -> bool {
        true
    }

    fn never_alive(_pid: i32) -> bool {
        false
    }

    fn request<'a>(path: &'a Path, agent: &'a str, token: &'a str) -> AcquireRequest<'a> {
        AcquireRequest {
            path,
            agent,
            token,
            purpose: Some("ui_click"),
            pid: std::process::id() as i32,
            ttl_ms: 30_000,
            wait_ms: 0,
            poll_ms: 20,
        }
    }

    #[test]
    fn first_acquirer_wins_and_second_is_told_to_wait() {
        let dir = temp_dir("basic");
        let path = dir.join("lease.json");

        let first = acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();
        assert!(matches!(first, Acquire::Acquired(_)));

        let second = acquire(&request(&path, "agent-b", "t2"), &always_alive).unwrap();
        match second {
            Acquire::Busy { holder, same_agent } => {
                assert_eq!(holder.agent, "agent-a");
                assert_eq!(holder.purpose.as_deref(), Some("ui_click"));
                assert!(!same_agent);
                // 建议重试时间必须是正数，否则等待方会立刻空转重试。
                assert!(Acquire::suggested_retry_ms(&holder) > 0);
            }
            other => panic!("应报忙碌，实际：{other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_only_works_for_the_owner() {
        let dir = temp_dir("release");
        let path = dir.join("lease.json");
        acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();

        assert!(!release(&path, "别人的 token"), "不该删掉别人的租约");
        assert!(current(&path, &always_alive).is_some());

        assert!(release(&path, "t1"));
        assert!(current(&path, &always_alive).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_budget_exhausts_then_reports_busy() {
        let dir = temp_dir("wait");
        let path = dir.join("lease.json");
        acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();

        let mut waiting = request(&path, "agent-b", "t2");
        waiting.wait_ms = 150;
        let started = std::time::Instant::now();
        let outcome = acquire(&waiting, &always_alive).unwrap();
        let elapsed = started.elapsed().as_millis();

        assert!(matches!(outcome, Acquire::Busy { .. }));
        // 必须真的等过，而不是立刻返回；也不该远超预算。
        assert!(elapsed >= 120, "等待时间过短：{elapsed}ms");
        assert!(elapsed < 2_000, "等待时间过长：{elapsed}ms");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_lease_is_taken_over() {
        let dir = temp_dir("expired");
        let path = dir.join("lease.json");

        let mut short = request(&path, "agent-a", "t1");
        short.ttl_ms = 1_000;
        acquire(&short, &always_alive).unwrap();

        // 手工把到期时间拨到过去，模拟持有者卡死。
        let mut info = peek(&path).unwrap();
        info.expires_at_ms = now_ms() - 1;
        fs::write(&path, serde_json::to_string(&info).unwrap()).unwrap();

        let outcome = acquire(&request(&path, "agent-b", "t2"), &always_alive).unwrap();
        assert!(matches!(outcome, Acquire::Acquired(_)), "过期租约应可接管");
        assert_eq!(peek(&path).unwrap().agent, "agent-b");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dead_holder_process_is_taken_over_without_waiting() {
        let dir = temp_dir("dead");
        let path = dir.join("lease.json");
        acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();

        let started = std::time::Instant::now();
        // 持有者进程已消失：即使 TTL 还很远也应立刻接管。
        let outcome = acquire(&request(&path, "agent-b", "t2"), &never_alive).unwrap();
        assert!(matches!(outcome, Acquire::Acquired(_)));
        assert!(started.elapsed().as_millis() < 500, "不该等待");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_owner_release_does_not_remove_new_holders_lease() {
        let dir = temp_dir("steal");
        let path = dir.join("lease.json");

        let mut short = request(&path, "agent-a", "t1");
        short.ttl_ms = 1_000;
        let guard = match acquire(&short, &always_alive).unwrap() {
            Acquire::Acquired(info) => LeaseGuard::new(path.clone(), info),
            Acquire::Busy { .. } => panic!("首次获取不该忙碌"),
        };

        // 模拟 agent-a 卡住，agent-b 接管。
        let mut info = peek(&path).unwrap();
        info.expires_at_ms = now_ms() - 1;
        fs::write(&path, serde_json::to_string(&info).unwrap()).unwrap();
        acquire(&request(&path, "agent-b", "t2"), &always_alive).unwrap();

        // agent-a 现在才释放：token 已不匹配，不应删掉 b 的租约。
        drop(guard);
        let survivor = peek(&path).expect("agent-b 的租约应仍在");
        assert_eq!(survivor.agent, "agent-b");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn guard_releases_on_drop() {
        let dir = temp_dir("guard");
        let path = dir.join("lease.json");

        let guard = match acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap() {
            Acquire::Acquired(info) => LeaseGuard::new(path.clone(), info),
            Acquire::Busy { .. } => panic!("首次获取不该忙碌"),
        };
        assert!(current(&path, &always_alive).is_some());
        drop(guard);
        assert!(current(&path, &always_alive).is_none(), "drop 后应释放");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_extends_only_own_lease() {
        let dir = temp_dir("refresh");
        let path = dir.join("lease.json");
        acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();

        assert!(!refresh(&path, "别人的", 60_000));
        let before = peek(&path).unwrap().expires_at_ms;
        assert!(refresh(&path, "t1", 60_000));
        assert!(peek(&path).unwrap().expires_at_ms > before);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_lease_file_is_treated_as_vacant() {
        let dir = temp_dir("corrupt");
        let path = dir.join("lease.json");
        fs::write(&path, "这不是 JSON").unwrap();

        assert!(peek(&path).is_none());
        let outcome = acquire(&request(&path, "agent-a", "t1"), &always_alive);
        // 损坏文件存在时 create_new 会失败，但接管路径会把它挪走并成功。
        assert!(matches!(outcome.unwrap(), Acquire::Acquired(_)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_agent_is_reported_so_caller_can_tell_apart_self_interference() {
        let dir = temp_dir("same");
        let path = dir.join("lease.json");
        acquire(&request(&path, "agent-a", "t1"), &always_alive).unwrap();

        let outcome = acquire(&request(&path, "agent-a", "t2"), &always_alive).unwrap();
        match outcome {
            Acquire::Busy { same_agent, .. } => assert!(same_agent, "应识别出是同一个 agent"),
            other => panic!("应报忙碌，实际：{other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod retry_hint_tests {
    use super::*;

    #[test]
    fn suggestion_is_a_poll_cadence_not_the_ttl() {
        // TTL 有 30 秒，但建议等待应该是一秒量级：对方多半几百毫秒就做完了。
        assert_eq!(Acquire::suggested_retry_for(30_000), 1_000);
        // 剩余很短时不要建议等更久。
        assert_eq!(Acquire::suggested_retry_for(120), 120);
        // 永不返回 0，否则调用方会空转。
        assert_eq!(Acquire::suggested_retry_for(0), 1);
    }
}
