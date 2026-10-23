pub type RequestId = u64;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenMode {
    #[default]
    Windowed,
    Borderless,
}
impl ScreenMode {
    pub fn from_setting(value: i64) -> Self {
        match value {
            1 | 2 => ScreenMode::Borderless,
            _ => ScreenMode::Windowed,
        }
    }
    pub fn as_setting(self) -> i64 {
        match self {
            ScreenMode::Windowed => 0,
            ScreenMode::Borderless => 2,
        }
    }
    pub fn is_fullscreen(self) -> bool {
        matches!(self, ScreenMode::Borderless)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuCommand {
    ReturnToTitle,
    ToggleMessagePanel,
    ToggleAuto,
    ToggleSkip,
    QuickSave,
    Save,
    Load,
    Config,
    ScreenBorderless,
    ScreenWindowed,
    Help,
    Website,
    Exit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenuState {
    pub panel_hidden: bool,
    pub panel_toggle_enabled: bool,
    pub auto_checked: bool,
    pub skip_checked: bool,
    pub save_load_enabled: bool,
    pub screen_mode: ScreenMode,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformRequest {
    SetScreenMode { mode: ScreenMode },
    RestoreWindowState { x: i32, y: i32, width: u32, height: u32, maximized: bool },
    QueryWindowGeometry,
    SetPointerCapture { enabled: bool },
    ShowContextMenu { request_id: RequestId, state: ContextMenuState },
    EditSaveComment { request_id: RequestId, initial: String, max_chars: u16 },
    #[cfg(target_arch = "wasm32")]
    PersistSave { save_name: String, bytes: Vec<u8> },
    OpenHelp,
    OpenUrl { url: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformResponse {
    ScreenModeApplied { mode: ScreenMode, ok: bool },
    WindowGeometry(WindowGeometry),
    WindowGeometryUnavailable,
    ContextMenuClosed { request_id: RequestId, command: Option<ContextMenuCommand> },
    SaveCommentClosed { request_id: RequestId, comment: Option<String> },
}

