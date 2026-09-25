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

/// 一个辅助功能元素的快照。
///
/// `path` 是从应用根元素到该元素的子索引链，例如 `[0, 1, 3]`。路径无状态：
/// 每次操作都从根重新走一遍解析，因此不需要在进程间保存元素句柄。
/// 树结构变化后旧路径会失效，此时应重新取树。
#[derive(Clone, Debug, PartialEq)]
pub struct ElementNode {
    pub path: Vec<usize>,
    /// 距根的层数，根为 0。
    pub depth: usize,
    pub role: String,
    pub subrole: Option<String>,
    pub title: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
    /// 屏幕点坐标。
    pub bounds: Option<Rect>,
    pub enabled: Option<bool>,
    pub focused: Option<bool>,
    /// 直接子元素数量，用来判断是否还有更深的内容。
    pub children: usize,
}

impl ElementNode {
    /// 路径的稳定文本形式，如 `0.1.3`。应用根元素为空串。
    pub fn path_text(&self) -> String {
        join_path(&self.path)
    }
}

/// 索引链转文本。空链（应用根元素）对应空串。
pub fn join_path(path: &[usize]) -> String {
    path.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// 解析 `0.1.3` 形式的路径。空串与非法段一律报错，避免静默取到错误元素。
pub fn parse_path(text: &str) -> std::result::Result<Vec<usize>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("空路径指应用根元素，它不是可操作的目标".to_string());
    }
    trimmed
        .split('.')
        .map(|segment| {
            segment
                .parse::<usize>()
                .map_err(|_| format!("路径段不是数字：{segment}"))
        })
        .collect()
}

/// 取树时的边界，避免整棵大树把输出撑爆。
#[derive(Clone, Copy, Debug)]
pub struct TreeLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_depth: 12,
            max_nodes: 200,
        }
    }
}

/// 元素支持的动作。名称与 macOS 的 AXAction 常量一致，便于排查。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementAction {
    /// 相当于点击。
    Press,
    /// 展开上下文菜单。
    ShowMenu,
    Increment,
    Decrement,
    Confirm,
    Cancel,
    Pick,
}

impl ElementAction {
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().replace(['_', '-'], "").as_str() {
            "press" | "click" => Some(ElementAction::Press),
            "showmenu" | "menu" => Some(ElementAction::ShowMenu),
            "increment" => Some(ElementAction::Increment),
            "decrement" => Some(ElementAction::Decrement),
            "confirm" => Some(ElementAction::Confirm),
            "cancel" => Some(ElementAction::Cancel),
            "pick" => Some(ElementAction::Pick),
            _ => None,
        }
    }

    /// 对应 macOS 的 AXAction 名。
    pub fn ax_name(&self) -> &'static str {
        match self {
            ElementAction::Press => "AXPress",
            ElementAction::ShowMenu => "AXShowMenu",
            ElementAction::Increment => "AXIncrement",
            ElementAction::Decrement => "AXDecrement",
            ElementAction::Confirm => "AXConfirm",
            ElementAction::Cancel => "AXCancel",
            ElementAction::Pick => "AXPick",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ElementAction::Press => "press",
            ElementAction::ShowMenu => "showMenu",
            ElementAction::Increment => "increment",
            ElementAction::Decrement => "decrement",
            ElementAction::Confirm => "confirm",
            ElementAction::Cancel => "cancel",
            ElementAction::Pick => "pick",
        }
    }
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

    /// 按名称找一个正在运行的应用，不改变前台。
    ///
    /// 与 `activate` 分开是为了避免「只想查元素却把窗口切了过去」。
    fn running_app(&self, _name: &str) -> Result<RunningApp> {
        Err(BackendError::Unsupported("running app lookup".into()))
    }

    /// 读取某进程的辅助功能元素树（拍平、广度优先）。
    ///
    /// 未实现该能力的平台返回 `Unsupported`，而不是空列表 —— 空列表会被误读成
    /// 「这个界面没有任何元素」。`window` 给定时只看该窗口对应的根元素。
    fn element_tree(
        &self,
        _pid: i32,
        _limits: &TreeLimits,
        _window: Option<u64>,
    ) -> Result<Vec<ElementNode>> {
        Err(BackendError::Unsupported("element tree".into()))
    }

    /// 对元素执行动作。返回实际投递的 AXAction 名，便于确认走的是哪条路径。
    fn element_act(
        &self,
        _pid: i32,
        _path: &[usize],
        _action: ElementAction,
    ) -> Result<String> {
        Err(BackendError::Unsupported("element action".into()))
    }

    /// 设置元素的值。
    fn element_set_value(&self, _pid: i32, _path: &[usize], _value: &str) -> Result<()> {
        Err(BackendError::Unsupported("element value".into()))
    }

    /// 列出元素支持的动作。列表为空表示该元素不可交互。
    fn element_actions(&self, _pid: i32, _path: &[usize]) -> Result<Vec<String>> {
        Err(BackendError::Unsupported("element actions".into()))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_text_and_parse_roundtrip() {
        for path in [vec![0usize], vec![0, 1, 3], vec![12, 0]] {
            let text = join_path(&path);
            assert_eq!(parse_path(&text).unwrap(), path, "往返失败：{text}");
        }
    }

    #[test]
    fn root_path_is_empty_and_not_operable() {
        // 应用根元素的路径是空串，它不能作为操作目标。
        assert_eq!(join_path(&[]), "");
        assert!(parse_path("").is_err());
        assert!(parse_path("   ").is_err());
    }

    #[test]
    fn malformed_paths_are_rejected() {
        assert!(parse_path("a.b").is_err());
        assert!(parse_path("0.x").is_err());
        assert!(parse_path("0..1").is_err());
        assert!(parse_path("-1").is_err());
    }

    #[test]
    fn element_action_names_match_macos() {
        assert_eq!(ElementAction::Press.ax_name(), "AXPress");
        assert_eq!(ElementAction::ShowMenu.ax_name(), "AXShowMenu");
        assert_eq!(ElementAction::parse("click"), Some(ElementAction::Press));
        assert_eq!(ElementAction::parse("show_menu"), Some(ElementAction::ShowMenu));
        assert!(ElementAction::parse("nope").is_none());
    }

    #[test]
    fn default_limits_are_bounded() {
        let limits = TreeLimits::default();
        assert!(limits.max_nodes > 0 && limits.max_depth > 0);
        // 默认值必须能挡住失控的树，否则 MCP 输出会被撑爆。
        assert!(limits.max_nodes <= 1000);
    }
}
