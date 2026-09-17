use super::api::{ContextMenuCommand, ContextMenuState};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject, EnumFontFamiliesExW,
    GetFontData, SelectObject, CLIP_DEFAULT_PRECIS, GDI_ERROR, HGDIOBJ, LOGFONTW,
    OUT_TT_ONLY_PRECIS, PROOF_QUALITY, SHIFTJIS_CHARSET, VARIABLE_PITCH,
};
use windows::Win32::UI::Controls::EM_LIMITTEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, SetFocus,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateDialogIndirectParamW, CreatePopupMenu, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetDlgItemTextW, GetMessageW, GetWindowLongPtrW,
    IsDialogMessageW, IsWindow, IsWindowVisible, PostMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetWindowPos, ShowWindow, TrackPopupMenuEx, TranslateMessage,
    BS_DEFPUSHBUTTON, DLGTEMPLATE, DS_SETFONT, ES_AUTOHSCROLL, GWLP_USERDATA,
    MENU_ITEM_FLAGS, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOW, SW_SHOWNORMAL,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_INITDIALOG,
    WM_NULL, WS_BORDER, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};
const IDC_COMMENT: i32 = 1003;
const IDOK_I: usize = 1;
const IDCANCEL_I: usize = 2;
const EM_LIMITTEXT_U32: u32 = EM_LIMITTEXT;
#[derive(Debug, Clone, Copy)]
pub struct WindowsPlatform {
    pub hwnd: HWND,
}
impl WindowsPlatform {
    pub fn new(hwnd: isize) -> Self {
        Self {
            hwnd: HWND(hwnd as *mut core::ffi::c_void),
        }
    }
    pub fn set_pointer_capture(&self, enabled: bool) {
        unsafe {
            if enabled {
                SetCapture(self.hwnd);
            } else {
                let _ = ReleaseCapture();
            }
        }
    }
    pub fn show_context_menu(
        &self,
        state: ContextMenuState,
    ) -> Option<ContextMenuCommand> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let add = |id: usize, text: &str, flags| {
                let wide = wide(text);
                let _ = AppendMenuW(menu, flags, id, PCWSTR(wide.as_ptr()));
            };
            add(1, "Return to title", MF_STRING);
            add(
                2,
                if state.panel_hidden {
                    "Show message panel"
                } else {
                    "Hide message panel"
                },
                if state.panel_toggle_enabled {
                    MF_STRING
                } else {
                    MF_STRING | MF_DISABLED | MF_GRAYED
                },
            );
            add(
                3,
                "Auto",
                MF_STRING
                    | if state.auto_checked { MF_CHECKED } else { MENU_ITEM_FLAGS(0) },
            );
            add(
                4,
                "Skip",
                MF_STRING
                    | if state.skip_checked { MF_CHECKED } else { MENU_ITEM_FLAGS(0) },
            );
            add(0, "", MF_SEPARATOR);
            let guarded = if state.save_load_enabled {
                MF_STRING
            } else {
                MF_STRING | MF_DISABLED | MF_GRAYED
            };
            add(5, "Quick save", guarded);
            add(6, "Save", guarded);
            add(7, "Load", guarded);
            add(8, "Config", MF_STRING);
            add(0, "", MF_SEPARATOR);
            if state.screen_mode.is_fullscreen() {
                add(12, "Screen size 100% (&W)", MF_STRING);
            } else {
                add(11, "Fullscreen (&F)", MF_STRING);
            }
            add(0, "", MF_SEPARATOR);
            add(9, "Help", MF_STRING);
            add(10, "Website", MF_STRING);
            add(13, "Exit", MF_STRING);
            let mut point = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenuEx(
                menu,
                TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0,
                point.x,
                point.y,
                self.hwnd,
                None,
            );
            let selected = command.0 as usize;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            command_from_id(selected)
        }
    }
    pub fn open_help(&self) {
        unsafe {
            let operation = wide("open");
            let file = wide("eden.chm");
            let _ = ShellExecuteW(
                Some(self.hwnd),
                PCWSTR(operation.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            );
        }
    }
    pub fn open_url(&self, url: &str) {
        unsafe {
            let operation = wide("open");
            let file = wide(url);
            let _ = ShellExecuteW(
                Some(self.hwnd),
                PCWSTR(operation.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            );
        }
    }
    pub fn edit_save_comment(&self, initial: &str, max_chars: u16) -> Option<String> {
        let mut context = Box::new(DialogContext {
            initial: initial.to_owned(),
            result: None,
            max_chars,
            done: false,
        });
        let template = DialogTemplate::new();
        unsafe {
            let dialog = match CreateDialogIndirectParamW(
                None,
                template.as_ptr(),
                Some(self.hwnd),
                Some(dialog_proc),
                LPARAM((&mut *context as *mut DialogContext) as isize),
            ) {
                Ok(dialog) if !dialog.0.is_null() => dialog,
                Ok(_) => {
                    log::error!(
                        "存档注释对话框创建返回空句柄，本次存档取消"
                    );
                    return None;
                }
                Err(error) => {
                    log::error!(
                        "存档注释对话框创建失败（{error}），本次存档取消"
                    );
                    return None;
                }
            };
            if !show_dialog(dialog) {
                log::error!(
                    "存档注释对话框无法显示，本次存档取消（避免游戏假死）"
                );
                let _ = DestroyWindow(dialog);
                return None;
            }
            log::info!("存档注释对话框已显示 (hwnd={:#x})", dialog.0 as usize);
            let _ = EnableWindow(self.hwnd, false);
            let mut message = MSG::default();
            while !context.done && GetMessageW(&mut message, None, 0, 0).0 > 0 {
                if IsDialogMessageW(dialog, &message).as_bool() {
                    continue;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let _ = EnableWindow(self.hwnd, true);
            if IsWindow(Some(dialog)).as_bool() {
                let _ = DestroyWindow(dialog);
            }
        }
        context.result.take()
    }
}
unsafe fn show_dialog(dialog: HWND) -> bool {
    for attempt in 0..3 {
        match attempt {
            0 => {
                let _ = ShowWindow(dialog, SW_SHOWNORMAL);
            }
            1 => {
                let _ = ShowWindow(dialog, SW_SHOW);
            }
            _ => {
                let _ = SetWindowPos(
                    dialog,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
            }
        }
        let _ = SetForegroundWindow(dialog);
        if IsWindowVisible(dialog).as_bool() {
            return true;
        }
    }
    false
}
pub fn double_click_limits() -> (std::time::Duration, f32, f32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    };
    unsafe {
        (
            std::time::Duration::from_millis(GetDoubleClickTime() as u64),
            (GetSystemMetrics(SM_CXDOUBLECLK).max(1) as f32) * 0.5,
            (GetSystemMetrics(SM_CYDOUBLECLK).max(1) as f32) * 0.5,
        )
    }
}
pub fn system_font_faces() -> Vec<String> {
    unsafe extern "system" fn collect_face(
        font: *const LOGFONTW,
        _metrics: *const windows::Win32::Graphics::Gdi::TEXTMETRICW,
        _kind: u32,
        param: LPARAM,
    ) -> i32 {
        if font.is_null() || param.0 == 0 {
            return 1;
        }
        let units = unsafe { &(*font).lfFaceName };
        let len = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
        let face = String::from_utf16_lossy(&units[..len]);
        if !face.is_empty() && !face.starts_with('@') {
            unsafe { &mut *(param.0 as *mut Vec<String>) }.push(face);
        }
        1
    }
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return Vec::new();
        }
        let mut query = LOGFONTW::default();
        query.lfCharSet = SHIFTJIS_CHARSET;
        let mut faces: Vec<String> = Vec::new();
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(collect_face),
            LPARAM((&mut faces as *mut Vec<String>) as isize),
            0,
        );
        let _ = DeleteDC(dc);
        faces.sort_by_key(|name| name.to_lowercase());
        faces.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        faces
    }
}
pub fn system_font_data(face: &str) -> anyhow::Result<Vec<u8>> {
    let face_w = wide(face);
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            anyhow::bail!("CreateCompatibleDC failed");
        }
        let font = CreateFontW(
            -22,
            0,
            0,
            0,
            500,
            0,
            0,
            0,
            SHIFTJIS_CHARSET,
            OUT_TT_ONLY_PRECIS,
            CLIP_DEFAULT_PRECIS,
            PROOF_QUALITY,
            VARIABLE_PITCH.0 as u32,
            PCWSTR(face_w.as_ptr()),
        );
        if font.0.is_null() {
            let _ = DeleteDC(dc);
            anyhow::bail!("CreateFontW failed for {face}");
        }
        let font_obj = HGDIOBJ(font.0);
        let old = SelectObject(dc, font_obj);
        let result = (|| {
            let size = GetFontData(dc, 0, 0, None, 0);
            if size == GDI_ERROR as u32 {
                anyhow::bail!("GetFontData size failed for {face}");
            }
            let mut data = vec![0u8; size as usize];
            let read = GetFontData(dc, 0, 0, Some(data.as_mut_ptr().cast()), size);
            if read == GDI_ERROR as u32 || read != size {
                anyhow::bail!("GetFontData read failed for {face}");
            }
            normalize_gdi_font_data(data)
        })();
        if !old.0.is_null() {
            let _ = SelectObject(dc, old);
        }
        let _ = DeleteObject(font_obj);
        let _ = DeleteDC(dc);
        result
    }
}
fn normalize_gdi_font_data(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    if ttf_parser::Face::parse(&data, 0).is_ok()
        || ttf_parser::fonts_in_collection(&data).is_some()
    {
        return Ok(data);
    }
    if data.len() < 12 {
        anyhow::bail!("GetFontData returned a truncated sfnt");
    }
    let table_count = u16::from_be_bytes([data[4], data[5]]) as usize;
    let directory_end = 12usize
        .checked_add(
            table_count
                .checked_mul(16)
                .ok_or_else(|| anyhow::anyhow!("sfnt table directory overflow"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("sfnt table directory overflow"))?;
    if directory_end > data.len() {
        anyhow::bail!("GetFontData returned a truncated sfnt table directory");
    }
    let head_record = (0..table_count)
        .map(|index| 12 + index * 16)
        .find(|&record| data[record..record + 4] == *b"head")
        .ok_or_else(|| anyhow::anyhow!("GetFontData sfnt has no head table record"))?;
    let head_offset = u32::from_be_bytes(
        data[head_record + 8..head_record + 12]
            .try_into()
            .expect("fixed-size sfnt field"),
    ) as usize;
    const HEAD_MAGIC: [u8; 4] = [0x5F, 0x0F, 0x3C, 0xF5];
    for (magic_offset, _) in data
        .windows(4)
        .enumerate()
        .filter(|(_, bytes)| *bytes == HEAD_MAGIC)
    {
        let Some(actual_head_offset) = magic_offset.checked_sub(12) else {
            continue;
        };
        let Some(base_offset) = head_offset.checked_sub(actual_head_offset) else {
            continue;
        };
        if base_offset == 0 {
            continue;
        }
        let mut normalized = data.clone();
        let mut valid = true;
        for index in 0..table_count {
            let record = 12 + index * 16;
            let absolute_offset = u32::from_be_bytes(
                data[record + 8..record + 12].try_into().expect("fixed-size sfnt field"),
            ) as usize;
            let length = u32::from_be_bytes(
                data[record + 12..record + 16].try_into().expect("fixed-size sfnt field"),
            ) as usize;
            let Some(relative_offset) = absolute_offset.checked_sub(base_offset) else {
                valid = false;
                break;
            };
            if relative_offset.checked_add(length).is_none_or(|end| end > data.len()) {
                valid = false;
                break;
            }
            normalized[record + 8..record + 12]
                .copy_from_slice(&(relative_offset as u32).to_be_bytes());
        }
        if valid && ttf_parser::Face::parse(&normalized, 0).is_ok() {
            return Ok(normalized);
        }
    }
    anyhow::bail!("could not normalize GetFontData TTC face offsets")
}
fn command_from_id(id: usize) -> Option<ContextMenuCommand> {
    Some(
        match id {
            1 => ContextMenuCommand::ReturnToTitle,
            2 => ContextMenuCommand::ToggleMessagePanel,
            3 => ContextMenuCommand::ToggleAuto,
            4 => ContextMenuCommand::ToggleSkip,
            5 => ContextMenuCommand::QuickSave,
            6 => ContextMenuCommand::Save,
            7 => ContextMenuCommand::Load,
            8 => ContextMenuCommand::Config,
            11 => ContextMenuCommand::ScreenBorderless,
            12 => ContextMenuCommand::ScreenWindowed,
            9 => ContextMenuCommand::Help,
            10 => ContextMenuCommand::Website,
            13 => ContextMenuCommand::Exit,
            _ => return None,
        },
    )
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
struct DialogContext {
    initial: String,
    result: Option<String>,
    max_chars: u16,
    done: bool,
}
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            let context = lparam.0 as *mut DialogContext;
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                context as isize,
            );
            if !context.is_null() {
                let context = &*context;
                if let Ok(edit) = windows::Win32::UI::WindowsAndMessaging::GetDlgItem(
                    Some(hwnd),
                    IDC_COMMENT,
                ) {
                    let _ = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                        edit,
                        EM_LIMITTEXT_U32,
                        Some(WPARAM(context.max_chars as usize)),
                        Some(LPARAM(0)),
                    );
                    let text = wide(&context.initial);
                    let _ = SetDlgItemTextW(hwnd, IDC_COMMENT, PCWSTR(text.as_ptr()));
                    let _ = SetFocus(Some(edit));
                }
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as usize;
            if id == IDOK_I {
                let context = dialog_context(hwnd);
                if !context.is_null() {
                    let mut text = vec![0u16; (* context).max_chars as usize + 1];
                    let len = GetDlgItemTextW(hwnd, IDC_COMMENT, &mut text) as usize;
                    text.truncate(len);
                    (*context).result = String::from_utf16(&text).ok();
                }
                let _ = finish_dialog(hwnd);
                1
            } else if id == IDCANCEL_I {
                let _ = finish_dialog(hwnd);
                1
            } else {
                0
            }
        }
        WM_CLOSE => {
            let _ = finish_dialog(hwnd);
            1
        }
        WM_DESTROY => {
            mark_done(hwnd);
            1
        }
        _ => 0,
    }
}
unsafe fn dialog_context(hwnd: HWND) -> *mut DialogContext {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogContext
}
unsafe fn finish_dialog(hwnd: HWND) -> windows::core::Result<()> {
    mark_done(hwnd);
    DestroyWindow(hwnd)
}
unsafe fn mark_done(hwnd: HWND) {
    let context = dialog_context(hwnd);
    if !context.is_null() {
        (*context).done = true;
    }
}
struct DialogTemplate {
    words: Vec<u32>,
    len: usize,
}
impl DialogTemplate {
    fn new() -> Self {
        let mut bytes = Vec::with_capacity(512);
        push_u32(
            &mut bytes,
            WS_POPUP.0 as u32 | WS_CAPTION.0 as u32 | WS_SYSMENU.0 as u32
                | DS_SETFONT as u32,
        );
        push_u32(&mut bytes, 0);
        push_u16(&mut bytes, 3);
        push_i16(&mut bytes, 10);
        push_i16(&mut bytes, 10);
        push_i16(&mut bytes, 280);
        push_i16(&mut bytes, 100);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_str(&mut bytes, "Save comment");
        push_u16(&mut bytes, 8);
        push_str(&mut bytes, "MS Shell Dlg");
        add_control(
            &mut bytes,
            WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32
                | WS_TABSTOP.0,
            10,
            12,
            260,
            18,
            IDC_COMMENT as u16,
            0x81,
            "",
        );
        add_control(
            &mut bytes,
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32,
            150,
            62,
            55,
            20,
            IDOK_I as u16,
            0x80,
            "OK",
        );
        add_control(
            &mut bytes,
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0,
            215,
            62,
            55,
            20,
            IDCANCEL_I as u16,
            0x80,
            "Cancel",
        );
        let len = bytes.len();
        let mut words = vec![0u32; (len + 3) / 4];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                words.as_mut_ptr() as *mut u8,
                len,
            );
        }
        Self { words, len }
    }
    fn as_ptr(&self) -> *const DLGTEMPLATE {
        debug_assert!(self.len <= self.words.len() * 4);
        self.words.as_ptr() as *const DLGTEMPLATE
    }
}
fn align4(bytes: &mut Vec<u8>) {
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
}
fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_str(bytes: &mut Vec<u8>, value: &str) {
    for unit in wide(value) {
        push_u16(bytes, unit);
    }
}
fn add_control(
    bytes: &mut Vec<u8>,
    style: u32,
    x: i16,
    y: i16,
    cx: i16,
    cy: i16,
    id: u16,
    class: u16,
    title: &str,
) {
    align4(bytes);
    push_u32(bytes, style);
    push_u32(bytes, 0);
    push_i16(bytes, x);
    push_i16(bytes, y);
    push_i16(bytes, cx);
    push_i16(bytes, cy);
    push_u16(bytes, id);
    push_u16(bytes, 0xffff);
    push_u16(bytes, class);
    push_str(bytes, title);
    push_u16(bytes, 0);
}

