//! 平台接入点。CLI 与 MCP 都从这里取后端与平台专属能力的解析函数，
//! 因此新增平台只需在这里补齐一个分支。

use uitap_core::backend::Modifier;

#[cfg(target_os = "macos")]
mod imp {
    use super::Modifier;
    use uitap_macos::MacBackend;

    pub type Current = MacBackend;

    pub fn open() -> Current {
        MacBackend::new()
    }

    /// 组合键 → 虚拟键码 + 修饰键。键码表是平台数据，故放在平台侧。
    pub fn parse_combo(text: &str) -> Option<(u16, Vec<Modifier>, String)> {
        uitap_macos::keys::parse_combo(text).map(|combo| (combo.key_code, combo.modifiers, combo.key))
    }

    pub fn capture_available() -> bool {
        uitap_macos::screencapture_available()
    }

    /// 标记主线程并准备任务通道。长驻进程（MCP server）必须在主线程调用一次。
    pub fn start_main_thread_service() -> bool {
        uitap_macos::mainthread::start()
    }

    /// 主线程事件循环，供 AppKit 调用落地。
    pub fn pump_main_thread() -> ! {
        uitap_macos::mainthread::pump_forever()
    }

    /// 请主线程事件循环退出。
    pub fn stop_main_thread_service() {
        uitap_macos::mainthread::shutdown()
    }

    /// 进程是否还在。用信号 0 探测：进程存在返回 0，权限不足返回 EPERM（也算存在）。
    /// 租约用它区分「持有者崩溃」与「持有者仍在忙」。
    pub fn process_alive(pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Modifier;
    use uitap_core::backend::{
        ActivateTarget, Backend, BackendError, CaptureTarget, DisplayInfo, MouseButton, Permissions,
        RawCapture, Result, RunningApp, WindowInfo,
    };
    use uitap_core::geom::Point;

    /// 未实现平台的占位后端：每个能力都明确报不支持，而不是返回空结果。
    #[derive(Debug, Default)]
    pub struct StubBackend;

    impl Backend for StubBackend {
        fn permissions(&self) -> Permissions {
            Permissions::default()
        }
        fn displays(&self) -> Result<Vec<DisplayInfo>> {
            Err(BackendError::Unsupported("display enumeration".into()))
        }
        fn windows(&self, _include_offscreen: bool) -> Result<Vec<WindowInfo>> {
            Err(BackendError::Unsupported("window enumeration".into()))
        }
        fn capture(&self, _target: &CaptureTarget, _out: &std::path::Path) -> Result<RawCapture> {
            Err(BackendError::Unsupported("screen capture".into()))
        }
        fn move_to(&self, _at: Point) -> Result<()> {
            Err(BackendError::Unsupported("pointer input".into()))
        }
        fn click(&self, _at: Point, _button: MouseButton, _count: u32) -> Result<()> {
            Err(BackendError::Unsupported("pointer input".into()))
        }
        fn drag(
            &self,
            _from: Point,
            _to: Point,
            _button: MouseButton,
            _duration_ms: u64,
        ) -> Result<()> {
            Err(BackendError::Unsupported("pointer input".into()))
        }
        fn scroll(&self, _at: Option<Point>, _dx: i32, _dy: i32) -> Result<()> {
            Err(BackendError::Unsupported("pointer input".into()))
        }
        fn type_text(&self, _text: &str, _delay_ms: u64) -> Result<()> {
            Err(BackendError::Unsupported("keyboard input".into()))
        }
        fn key(&self, _key_code: u16, _modifiers: &[Modifier]) -> Result<()> {
            Err(BackendError::Unsupported("keyboard input".into()))
        }
        fn activate(&self, _target: &ActivateTarget) -> Result<RunningApp> {
            Err(BackendError::Unsupported("app activation".into()))
        }
        fn frontmost(&self) -> Result<RunningApp> {
            Err(BackendError::Unsupported("frontmost app".into()))
        }
    }

    pub type Current = StubBackend;

    pub fn open() -> Current {
        StubBackend
    }

    pub fn parse_combo(_text: &str) -> Option<(u16, Vec<Modifier>, String)> {
        None
    }

    pub fn capture_available() -> bool {
        false
    }

    pub fn start_main_thread_service() -> bool {
        false
    }

    /// 无平台调用需要主线程，因此立刻返回。
    pub fn pump_main_thread() -> ! {
        std::process::exit(0)
    }

    pub fn stop_main_thread_service() {}

    /// 无平台实现时保守认为是活的，让租约只靠到期时间回收。
    pub fn process_alive(pid: i32) -> bool {
        pid > 0
    }
}

pub use imp::{
    capture_available, open, parse_combo, process_alive, pump_main_thread, start_main_thread_service,
    stop_main_thread_service, Current,
};

/// 平台名，用于 `doctor` 与错误信息。
pub const fn name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

/// 该平台是否已有可用实现。
pub const fn implemented() -> bool {
    cfg!(target_os = "macos")
}
