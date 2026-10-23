use crate::{bytes, platform_warning, Failure, Host};
use shinki_emu_abi as abi;
use eden_engine::{
    ContextMenuCommand as Cmd, Engine, PlatformRequest, PlatformResponse, ScreenMode,
};

fn request(host: &Host, kind: u32, id: u64) -> abi::WindowRequest {
    abi::WindowRequest {
        session_epoch: host.epoch,
        context_epoch: 1,
        request_id: id,
        kind,
        flags: 0,
        text: abi::Bytes::EMPTY,
        pixels: abi::Bytes::EMPTY,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        autohide_ms: 0,
        buttons: std::ptr::null(),
        button_count: 0,
        caption: abi::Bytes::EMPTY,
        initial_text: abi::Bytes::EMPTY,
    }
}
fn unsupported(operation: u64) -> Failure {
    Failure {
        status: abi::UNSUPPORTED,
        code: "REMINORI_PLATFORM_UNSUPPORTED",
        message: format!("host does not provide native platform operation {operation}"),
    }
}
fn call(
    host: &Host,
    request: &abi::WindowRequest,
    text: &mut [u8],
) -> Result<abi::WindowResult, Failure> {
    if host.cancelled() {
        return Err(Failure::status(abi::CANCELLED));
    }
    let callback = host
        .services
        .window
        .request
        .ok_or_else(|| unsupported(u64::from(request.kind)))?;
    let mut result = abi::WindowResult {
        selected_button: 0,
        reserved: 0,
        valid_fields: 0,
        fullscreen: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        input: abi::TextBuffer {
            data: text.as_mut_ptr(),
            capacity: text.len(),
            len: 0,
        },
        visible: 0,
        minimized: 0,
    };
    let mut error = abi::Error::default();
    let status = unsafe {
        callback(
            host.services.window.context,
            request,
            &mut result,
            &mut error,
        )
    };
    if status != abi::OK {
        let code = error
            .code
            .get(..error.code_len as usize)
            .and_then(|s| std::str::from_utf8(s).ok())
            .unwrap_or("invalid host error code");
        let message = error
            .message
            .get(..error.message_len as usize)
            .and_then(|s| std::str::from_utf8(s).ok())
            .unwrap_or("invalid host error message");
        return Err(Failure {
            status,
            code: "REMINORI_HOST_WINDOW",
            message: format!(
                "window operation {} returned status {status}: {code}: {message}",
                request.kind
            ),
        });
    }
    if host.cancelled() {
        return Err(Failure::status(abi::CANCELLED));
    }
    if result.reserved != 0
        || result.input.data != text.as_mut_ptr()
        || result.input.capacity != text.len()
        || result.input.len > text.len()
    {
        return Err(Failure::invalid(
            "host returned invalid window result storage",
        ));
    }
    Ok(result)
}
fn best_effort(host: &Host, request: &abi::WindowRequest) -> Option<abi::WindowResult> {
    match call(host, request, &mut []) {
        Ok(result) => Some(result),
        Err(failure) => {
            platform_warning(u64::from(request.kind), &failure);
            None
        }
    }
}
fn modal_selection(
    result: &abi::WindowResult,
    buttons: &[abi::WindowButton],
) -> Result<u32, Failure> {
    if result.valid_fields & abi::WINDOW_RESULT_MODAL == 0
        || !buttons
            .iter()
            .any(|button| button.id == result.selected_button)
    {
        return Err(Failure::invalid(
            "host modal result is missing or names an unknown button",
        ));
    }
    Ok(result.selected_button)
}
pub fn pump(engine: &mut Engine, host: &Host) -> Result<(), Failure> {
    for operation in engine.drain_platform_requests() {
        if host.cancelled() {
            return Err(Failure::status(abi::CANCELLED));
        }
        match operation {
            PlatformRequest::SetScreenMode { mode } => {
                let mut req = request(host, abi::WINDOW_FULLSCREEN, 0);
                req.flags = if mode == ScreenMode::Windowed {
                    abi::FULLSCREEN_WINDOWED
                } else {
                    abi::FULLSCREEN_BORDERLESS
                };
                let result = best_effort(host, &req);
                let ok = result.as_ref().is_some_and(|r| {
                    r.valid_fields & abi::WINDOW_RESULT_MODE != 0 && r.fullscreen == req.flags
                });
                if result.is_some() && !ok {
                    platform_warning(
                        u64::from(req.kind),
                        &Failure::invalid("host did not confirm requested screen mode"),
                    );
                }
                engine.platform_response(PlatformResponse::ScreenModeApplied { mode, ok });
            }
            PlatformRequest::RestoreWindowState {
                x,
                y,
                width,
                height,
                ..
            } => {
                let mut req = request(host, abi::WINDOW_SIZE, 0);
                req.width = width;
                req.height = height;
                let _ = best_effort(host, &req);
                req.kind = abi::WINDOW_POSITION;
                req.x = x;
                req.y = y;
                let _ = best_effort(host, &req);
            }
            PlatformRequest::QueryWindowGeometry => {
                // The host owns output geometry. Its ABI does not report the
                // maximized flag required by native save_window_geometry; do
                // not persist an invented observation.
                platform_warning(
                    u64::from(abi::WINDOW_QUERY_STATE),
                    &unsupported(u64::from(abi::WINDOW_QUERY_STATE)),
                );
                engine.platform_response(PlatformResponse::WindowGeometryUnavailable);
            }
            PlatformRequest::ShowContextMenu { request_id, state } => {
                let mut options = vec![
                    ("Return to title", Cmd::ReturnToTitle),
                    ("Settings", Cmd::Config),
                ];
                if state.panel_toggle_enabled {
                    options.push(("Toggle message panel", Cmd::ToggleMessagePanel));
                }
                if state.save_load_enabled {
                    options.extend([
                        ("Quick save", Cmd::QuickSave),
                        ("Save", Cmd::Save),
                        ("Load", Cmd::Load),
                        ("Toggle auto", Cmd::ToggleAuto),
                        ("Toggle skip", Cmd::ToggleSkip),
                    ]);
                }
                options.push(("Exit", Cmd::Exit));
                let mut buttons: Vec<abi::WindowButton> = options
                    .iter()
                    .enumerate()
                    .map(|(i, (label, _))| abi::WindowButton {
                        id: i as u32 + 1,
                        reserved: 0,
                        label: bytes(label),
                    })
                    .collect();
                buttons.push(abi::WindowButton {
                    id: 0,
                    reserved: 0,
                    label: bytes("Cancel"),
                });
                let mut req = request(host, abi::WINDOW_MODAL, request_id);
                req.caption = bytes("ReMinori");
                req.text = bytes("Game menu");
                req.buttons = buttons.as_ptr();
                req.button_count = buttons.len();
                let result = call(host, &req, &mut [])?;
                let selected = modal_selection(&result, &buttons)?;
                let command = if selected == 0 {
                    None
                } else {
                    Some(options[(selected - 1) as usize].1)
                };
                engine.platform_response(PlatformResponse::ContextMenuClosed {
                    request_id,
                    command,
                });
            }
            PlatformRequest::EditSaveComment {
                request_id,
                initial,
                max_chars,
            } => {
                let buttons = [
                    abi::WindowButton {
                        id: 1,
                        reserved: 0,
                        label: bytes("Save"),
                    },
                    abi::WindowButton {
                        id: 0,
                        reserved: 0,
                        label: bytes("Cancel"),
                    },
                ];
                let mut req = request(host, abi::WINDOW_MODAL, request_id);
                req.flags = abi::MODAL_TEXT_INPUT;
                req.caption = bytes("Save comment");
                req.initial_text = abi::Bytes::new(initial.as_bytes());
                req.buttons = buttons.as_ptr();
                req.button_count = buttons.len();
                let mut text = vec![0; abi::MAX_MODAL_TEXT];
                let result = call(host, &req, &mut text)?;
                let comment = match modal_selection(&result, &buttons)? {
                    0 => None,
                    1 => Some(
                        std::str::from_utf8(&text[..result.input.len])
                            .map_err(|_| Failure::invalid("host save comment is not UTF-8"))?
                            .chars()
                            .take(max_chars as usize)
                            .collect(),
                    ),
                    _ => unreachable!("modal selection validates the explicit button set"),
                };
                engine.platform_response(PlatformResponse::SaveCommentClosed {
                    request_id,
                    comment,
                });
            }
            PlatformRequest::SetPointerCapture { .. } => {
                platform_warning(0x10001, &unsupported(0x10001))
            }
            PlatformRequest::OpenHelp => platform_warning(0x10002, &unsupported(0x10002)),
            PlatformRequest::OpenUrl { .. } => platform_warning(0x10003, &unsupported(0x10003)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    unsafe extern "C" fn not_cancelled(_: *mut std::ffi::c_void, _: u64) -> u32 {
        0
    }
    fn host() -> Host {
        Host {
            epoch: 1,
            local_cancel: Arc::new(AtomicBool::new(false)),
            services: abi::HostServices {
                header: abi::Header::of::<abi::HostServices>(),
                text: abi::TextService {
                    context: std::ptr::null_mut(),
                    request: None,
                    acknowledge: None,
                    invalidate: None,
                },
                window: abi::WindowService {
                    context: std::ptr::null_mut(),
                    request: None,
                    invalidate: None,
                },
                cancellation: abi::CancellationService {
                    context: std::ptr::null_mut(),
                    is_cancelled: Some(not_cancelled),
                },
            },
        }
    }
    #[test]
    fn absent_modal_service_is_an_explicit_unsupported_error() {
        let host = host();
        let failure = call(&host, &request(&host, abi::WINDOW_MODAL, 1), &mut []).unwrap_err();
        assert_eq!(failure.status, abi::UNSUPPORTED);
    }
    #[test]
    fn only_a_valid_explicit_cancel_button_is_a_cancel_answer() {
        let buttons = [abi::WindowButton {
            id: 0,
            reserved: 0,
            label: bytes("Cancel"),
        }];
        let mut result = abi::WindowResult {
            selected_button: 0,
            reserved: 0,
            valid_fields: 0,
            fullscreen: 0,
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            visible: 0,
            minimized: 0,
            input: abi::TextBuffer {
                data: std::ptr::null_mut(),
                capacity: 0,
                len: 0,
            },
        };
        assert!(modal_selection(&result, &buttons).is_err());
        result.valid_fields = abi::WINDOW_RESULT_MODAL;
        assert_eq!(modal_selection(&result, &buttons).unwrap(), 0);
        result.selected_button = 99;
        assert!(modal_selection(&result, &buttons).is_err());
    }
}
