//! 主线程任务执行器。
//!
//! AppKit 的部分调用（尤其 `NSRunningApplication` 的激活）只有在本进程主线程上执行才会落地：
//! 从工作线程发起的激活请求会被系统搁置，无论怎么轮询都观察不到结果。
//! 因此长驻进程（MCP server）需要把主线程留给这类调用。
//!
//! 用法：主线程调用 [`start`]，随后用 [`pump_forever`] 驱动；工作线程用 [`run_on_main`] 投递任务。
//! 没有启动服务时 [`run_on_main`] 返回 `None`，调用方原地执行即可 —— 这在短命进程（CLI）里是对的，
//! 因为 CLI 的调用本来就发生在主线程。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;

type Job = Box<dyn FnOnce() + Send + 'static>;

static JOBS: OnceLock<SyncSender<Job>> = OnceLock::new();
static INBOX: OnceLock<Mutex<Option<Receiver<Job>>>> = OnceLock::new();
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// 空闲时每个事件循环轮次的时长。
const IDLE_SPIN_SECONDS: f64 = 0.02;

/// 记录主线程标识并准备任务通道。必须在主线程调用，且只能成功一次。
pub fn start() -> bool {
    let (tx, rx) = sync_channel::<Job>(64);
    if JOBS.set(tx).is_err() {
        return false;
    }
    if INBOX.set(Mutex::new(Some(rx))).is_err() {
        return false;
    }
    let _ = MAIN_THREAD.set(std::thread::current().id());
    SHUTDOWN.store(false, Ordering::SeqCst);
    true
}

pub fn is_main_thread() -> bool {
    MAIN_THREAD
        .get()
        .map(|id| *id == std::thread::current().id())
        .unwrap_or(false)
}

/// 请主线程在空闲后退出事件循环。
pub fn shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// 在主线程上同步执行闭包并取回结果。
///
/// 已经是主线程、或服务未启动时返回 `None`，由调用方原地执行。
pub fn run_on_main<T, F>(task: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if is_main_thread() {
        return None;
    }
    let sender = JOBS.get()?;
    let (reply_tx, reply_rx) = sync_channel(1);
    let job: Job = Box::new(move || {
        let _ = reply_tx.send(task());
    });
    if sender.send(job).is_err() {
        return None;
    }
    reply_rx.recv().ok()
}

/// 处理一个待执行任务。没有任务时让出 run loop，供系统派发事件。返回是否处理了任务。
pub fn pump_once() -> bool {
    let job = INBOX
        .get()
        .and_then(|lock| lock.lock().ok())
        .and_then(|mut guard| guard.as_mut().map(|rx| rx.try_recv()));

    match job {
        Some(Ok(task)) => {
            task();
            true
        }
        Some(Err(TryRecvError::Disconnected)) => std::process::exit(0),
        _ => {
            crate::spin_run_loop(IDLE_SPIN_SECONDS);
            false
        }
    }
}

/// 主线程事件循环。收到 shutdown 或通道断开时结束进程。
pub fn pump_forever() -> ! {
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            std::process::exit(0);
        }
        pump_once();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// 状态是进程级的：启动只能成功一次，所以整个生命周期必须在同一个测试里按序验证，
    /// 不能拆成多个并行用例。
    #[test]
    fn lifecycle() {
        // 1) 未启动：调用方应原地执行。
        assert!(!is_main_thread(), "启动前不应被视为主线程");
        let inline: Option<u32> = run_on_main(|| 7);
        assert!(inline.is_none(), "服务未启动时应返回 None");

        // 2) 启动，并且只能成功一次。
        assert!(start(), "首次启动应成功");
        assert!(is_main_thread(), "start 必须由主线程调用");
        assert!(!start(), "重复启动应返回 false 而不是静默重置");

        // 3) 工作线程投递的任务应在主线程执行并取回结果。
        let counter = Arc::new(AtomicUsize::new(0));
        let ran_on_main = Arc::new(AtomicBool::new(false));
        let handle = {
            let counter = counter.clone();
            let ran_on_main = ran_on_main.clone();
            std::thread::spawn(move || {
                run_on_main(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    ran_on_main.store(is_main_thread(), Ordering::SeqCst);
                    "done".to_string()
                })
            })
        };

        // 主线程跑事件循环，直到任务被取走处理。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while counter.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            pump_once();
        }

        let reply = handle.join().unwrap();
        assert_eq!(reply.as_deref(), Some("done"));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(ran_on_main.load(Ordering::SeqCst), "任务应在主线程执行");
    }
}
