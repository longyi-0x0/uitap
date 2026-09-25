//! 平台后端契约。core 只定义类型与 trait，不含任何平台调用。

use crate::geom::{Point, Rect};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Permissions {
    /// 屏幕录制：缺失则截图失败。
    pub screen_recording: bool,
    /// 辅助功能：缺失则输入合成静默失效。
    pub accessibility: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    pub index: usize,
    pub id: u64,
    /// 全局点坐标。
    pub bounds: Rect,
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub main: bool,
}

impl DisplayInfo {
    pub fn scale(&self) -> f64 {
        if self.bounds.w > 0.0 {
            self.pixel_width as f64 / self.bounds.w
        } else {
            1.0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub id: u64,
    pub pid: i32,
    pub app: String,
    pub title: String,
    /// 全局点坐标。
    pub bounds: Rect,
    pub layer: i32,
    pub onscreen: bool,
    /// 前后叠放序号，0 为最前。
    pub z: usize,
}

impl WindowInfo {
    pub fn area(&self) -> f64 {
        self.bounds.area()
    }

    /// 大窗口优先；同尺寸时最前的在前。与 Swift 版排序一致。
    pub fn ordered(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.area()
            .partial_cmp(&a.area())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.z.cmp(&b.z))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CaptureTarget {
    Screen { display_index: Option<usize> },
    Region(Rect),
    Window(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    Screen,
    Region,
    Window,
}

impl CaptureMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            CaptureMode::Screen => "screen",
            CaptureMode::Region => "region",
            CaptureMode::Window => "window",
        }
    }
}

/// 一次截图的产物。PNG 已由后端按最终像素尺寸写盘，这里只回传锚点所需信息。
#[derive(Clone, Debug)]
pub struct RawCapture {
    pub path: std::path::PathBuf,
    pub pixel_width: usize,
    pub pixel_height: usize,
    /// 该图左上角对应的全局点。
    pub origin: Point,
    /// 该图覆盖的点尺寸。
    pub point_width: f64,
    pub point_height: f64,
    pub mode: CaptureMode,
    pub window_id: Option<u64>,
}

impl RawCapture {
    /// 像素宽除以点宽，用最终图像尺寸反推，因此缩放过的图依然成立。
    pub fn scale(&self) -> f64 {
        if self.point_width > 0.0 {
            self.pixel_width as f64 / self.point_width
        } else {
            1.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "left" => Some(MouseButton::Left),
            "right" => Some(MouseButton::Right),
            "middle" | "center" => Some(MouseButton::Middle),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActivateTarget {
    App(String),
    Pid(i32),
    Window(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub app: String,
    pub pid: i32,
    pub bundle_id: String,
    /// 该应用当前是否在前台。
    pub frontmost: bool,
}

/// 平台能力的失败一律带上可操作的说明，不返回静默的空结果。
#[derive(Debug)]
pub enum BackendError {
    Unsupported(String),
    Failed(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Unsupported(m) => write!(f, "unsupported: {m}"),
            BackendError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for BackendError {}

pub type Result<T> = std::result::Result<T, BackendError>;

/// 平台后端。观测与输入合并为一个 trait，避免两套生命周期。
pub trait Backend {
    fn permissions(&self) -> Permissions;

    fn displays(&self) -> Result<Vec<DisplayInfo>>;

    /// `include_offscreen` 为真时包含不在当前工作区的窗口。
    fn windows(&self, include_offscreen: bool) -> Result<Vec<WindowInfo>>;

    /// 截取一次画面，按最终像素尺寸写出 PNG，并返回坐标锚点所需信息。
    fn capture(&self, target: &CaptureTarget, out: &std::path::Path) -> Result<RawCapture>;

    fn move_to(&self, at: Point) -> Result<()>;

    fn click(&self, at: Point, button: MouseButton, count: u32) -> Result<()>;

    fn drag(&self, from: Point, to: Point, button: MouseButton, duration_ms: u64) -> Result<()>;

    fn scroll(&self, at: Option<Point>, dx: i32, dy: i32) -> Result<()>;

    /// 逐段键入文本，`delay_ms` 为 0 时整串投递。
    fn type_text(&self, text: &str, delay_ms: u64) -> Result<()>;

    /// `key_code` 为平台虚拟键码。
    fn key(&self, key_code: u16, modifiers: &[Modifier]) -> Result<()>;

    fn activate(&self, target: &ActivateTarget) -> Result<RunningApp>;

    fn frontmost(&self) -> Result<RunningApp>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Cmd,
    Shift,
    Opt,
    Ctrl,
    Fn,
}

impl Modifier {
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "⌘" | "win" | "super" => Some(Modifier::Cmd),
            "shift" | "⇧" => Some(Modifier::Shift),
            "opt" | "option" | "alt" | "⌥" => Some(Modifier::Opt),
            "ctrl" | "control" | "⌃" => Some(Modifier::Ctrl),
            "fn" => Some(Modifier::Fn),
            _ => None,
        }
    }
}
