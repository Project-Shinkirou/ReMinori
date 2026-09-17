use eden_engine::{Engine, PlatformRequest, PlatformResponse, ScreenMode, WindowGeometry};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Window};
fn apply_screen_mode(window: &Window, mode: ScreenMode) -> bool {
    match mode {
        ScreenMode::Windowed => {
            window.set_fullscreen(None);
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(1024, 640));
            true
        }
        ScreenMode::Borderless => {
            let Some(monitor) = window.current_monitor() else {
                return false;
            };
            window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
            true
        }
    }
}
fn restore_window_state(
    window: &Window,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
) {
    window.set_maximized(false);
    window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
    let outer = window.outer_size();
    let inner = window.inner_size();
    let frame_w = outer.width.saturating_sub(inner.width);
    let frame_h = outer.height.saturating_sub(inner.height);
    let _ = window
        .request_inner_size(
            winit::dpi::PhysicalSize::new(
                width.saturating_sub(frame_w),
                height.saturating_sub(frame_h),
            ),
        );
    window.set_maximized(maximized);
}
fn current_window_geometry(window: &Window) -> WindowGeometry {
    let position = window.outer_position().ok();
    let size = window.outer_size();
    WindowGeometry {
        x: position.map_or(0, |p| p.x),
        y: position.map_or(0, |p| p.y),
        width: size.width,
        height: size.height,
        maximized: window.is_maximized(),
    }
}
pub fn dispatch(engine: &mut Engine, window: &Window) {
    #[cfg(windows)]
    let platform = windows_platform(window);
    for request in engine.drain_platform_requests() {
        match request {
            PlatformRequest::SetScreenMode { mode } => {
                let ok = apply_screen_mode(window, mode);
                engine
                    .platform_response(PlatformResponse::ScreenModeApplied {
                        mode,
                        ok,
                    });
            }
            PlatformRequest::RestoreWindowState { x, y, width, height, maximized } => {
                restore_window_state(window, x, y, width, height, maximized)
            }
            PlatformRequest::QueryWindowGeometry => {
                engine
                    .platform_response(
                        PlatformResponse::WindowGeometry(current_window_geometry(window)),
                    )
            }
            #[cfg(windows)]
            PlatformRequest::SetPointerCapture { enabled } => {
                if let Some(platform) = &platform {
                    platform.set_pointer_capture(enabled);
                }
            }
            #[cfg(windows)]
            PlatformRequest::ShowContextMenu { request_id, state } => {
                let command = platform
                    .as_ref()
                    .and_then(|platform| platform.show_context_menu(state));
                engine
                    .platform_response(PlatformResponse::ContextMenuClosed {
                        request_id,
                        command,
                    });
            }
            #[cfg(windows)]
            PlatformRequest::EditSaveComment { request_id, initial, max_chars } => {
                let comment = platform
                    .as_ref()
                    .and_then(|platform| {
                        platform.edit_save_comment(&initial, max_chars)
                    });
                engine
                    .platform_response(PlatformResponse::SaveCommentClosed {
                        request_id,
                        comment,
                    });
            }
            #[cfg(windows)]
            PlatformRequest::OpenHelp => {
                if let Some(platform) = &platform {
                    platform.open_help();
                }
            }
            #[cfg(windows)]
            PlatformRequest::OpenUrl { url } => {
                if let Some(platform) = &platform {
                    platform.open_url(&url);
                }
            }
            #[cfg(not(windows))]
            PlatformRequest::ShowContextMenu { request_id, .. } => {
                engine
                    .platform_response(PlatformResponse::ContextMenuClosed {
                        request_id,
                        command: None,
                    })
            }
            #[cfg(not(windows))]
            PlatformRequest::EditSaveComment { request_id, .. } => {
                engine
                    .platform_response(PlatformResponse::SaveCommentClosed {
                        request_id,
                        comment: None,
                    })
            }
            #[cfg(not(windows))]
            _ => {}
        }
    }
}
#[cfg(windows)]
fn windows_platform(
    window: &Window,
) -> Option<eden_engine::platform::windows::WindowsPlatform> {
    let handle = window.window_handle().ok()?.as_raw();
    let RawWindowHandle::Win32(handle) = handle else {
        return None;
    };
    Some(eden_engine::platform::windows::WindowsPlatform::new(handle.hwnd.get()))
}
