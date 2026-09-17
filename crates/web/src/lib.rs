#[cfg(target_arch = "wasm32")]
mod audio;
mod browser_fs;
mod storage;
#[cfg(any(target_arch = "wasm32", test))]
mod transfer;
#[cfg(target_arch = "wasm32")]
mod video;
#[cfg(target_arch = "wasm32")]
mod wasm {
    use eden_engine::render::gpu::Renderer;
    use eden_engine::ContextMenuCommand;
    use eden_engine::{
        ButtonState, CgSeen, Engine, EngineServices, FontSource, InputEvent, InputKey,
        PlatformRequest, PlatformResponse, PointerButton, Settings, StorageBackend,
        VideoBackend, Viewport,
    };
    use js_sys::{Object, Promise, Reflect, Uint8Array};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::sync::Arc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{prelude::*, JsCast};
    use wasm_bindgen_futures::{spawn_local, JsFuture};
    use web_sys::{Event, HtmlButtonElement, HtmlCanvasElement};
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::{
        ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
    };
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::keyboard::{Key, NamedKey};
    use winit::platform::web::{
        EventLoopExtWebSys, WindowAttributesExtWebSys, WindowExtWebSys,
    };
    use winit::window::{Window, WindowId};
    const SMOKE_COLOR: [f64; 4] = [0.93, 0.93, 0.93, 1.0];
    struct BrowserLogger;
    static BROWSER_LOGGER: BrowserLogger = BrowserLogger;
    impl log::Log for BrowserLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Debug
        }
        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let message = JsValue::from_str(
                &format!("[{} {}] {}", record.level(), record.target(), record.args()),
            );
            match record.level() {
                log::Level::Error => web_sys::console::error_1(&message),
                log::Level::Warn => web_sys::console::warn_1(&message),
                log::Level::Info => web_sys::console::info_1(&message),
                log::Level::Debug | log::Level::Trace => {
                    web_sys::console::debug_1(&message)
                }
            }
        }
        fn flush(&self) {}
    }
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = edenShowContextMenu)]
        fn show_context_menu(state: &JsValue) -> Promise;
        #[wasm_bindgen(js_name = edenEditSaveComment)]
        fn edit_save_comment(initial: &str, max_chars: u16) -> Promise;
        #[wasm_bindgen(js_name = edenPickSaveFile)]
        fn pick_save_file() -> Promise;
        #[wasm_bindgen(js_name = edenChooseImportSlot)]
        fn choose_import_slot(file_name: &str, suggested_slot: usize) -> Promise;
        #[wasm_bindgen(js_name = edenConfirmSaveOverwrite)]
        fn confirm_save_overwrite(slot: usize) -> bool;
        #[wasm_bindgen(js_name = edenDownloadBytes)]
        fn download_bytes(file_name: &str, mime_type: &str, bytes: &JsValue);
        #[wasm_bindgen(js_name = edenCaptureThumbnail)]
        fn capture_thumbnail(canvas: &HtmlCanvasElement) -> Promise;
        #[wasm_bindgen(js_name = edenApplyFullscreen)]
        fn apply_fullscreen(canvas: &HtmlCanvasElement, enabled: bool);
    }
    struct TouchTrack {
        finger: u64,
        start: (f32, f32),
        last: (f32, f32),
        started_at: web_time::Instant,
        consumed: bool,
    }
    impl TouchTrack {
        const TAP_MAX: std::time::Duration = std::time::Duration::from_millis(400);
        const LONG_PRESS: std::time::Duration = std::time::Duration::from_millis(500);
        const SLOP: f32 = 8.0;
        fn moved_beyond_slop(&self) -> bool {
            (self.last.0 - self.start.0).abs() > Self::SLOP
                || (self.last.1 - self.start.1).abs() > Self::SLOP
        }
        fn long_press_due(&self, now: web_time::Instant) -> bool {
            !self.consumed && !self.moved_beyond_slop()
                && now.saturating_duration_since(self.started_at) >= Self::LONG_PRESS
        }
        fn is_tap(&self, now: web_time::Instant) -> bool {
            !self.consumed && !self.moved_beyond_slop()
                && now.saturating_duration_since(self.started_at) <= Self::TAP_MAX
        }
    }
    #[derive(Default)]
    struct BrowserInput {
        cursor_physical: Option<(f32, f32)>,
        last_press: Option<(PointerButton, web_time::Instant, f32, f32)>,
        wheel_x: f32,
        wheel_y: f32,
    }
    impl BrowserInput {
        fn pointer_button(
            &mut self,
            button: MouseButton,
            state: ElementState,
        ) -> InputEvent {
            let button = match button {
                MouseButton::Left => PointerButton::Left,
                MouseButton::Right => PointerButton::Right,
                MouseButton::Middle => PointerButton::Middle,
                MouseButton::Back => PointerButton::Back,
                MouseButton::Forward => PointerButton::Forward,
                MouseButton::Other(value) => PointerButton::Other(value),
            };
            let click_count = if state == ElementState::Pressed {
                let now = web_time::Instant::now();
                let (x, y) = self.cursor_physical.unwrap_or_default();
                let double = self
                    .last_press
                    .is_some_and(|(old_button, at, old_x, old_y)| {
                        old_button == button
                            && now.saturating_duration_since(at)
                                <= std::time::Duration::from_millis(500)
                            && (x - old_x).abs() <= 4.0 && (y - old_y).abs() <= 4.0
                    });
                self.last_press = (!double).then_some((button, now, x, y));
                if double { 2 } else { 1 }
            } else {
                1
            };
            InputEvent::PointerButton {
                button,
                state: button_state(state),
                click_count,
            }
        }
        fn wheel(&mut self, delta: MouseScrollDelta) -> Option<InputEvent> {
            match delta {
                MouseScrollDelta::LineDelta(x, y) => {
                    Some(InputEvent::WheelSteps { x, y })
                }
                MouseScrollDelta::PixelDelta(position) => {
                    self.wheel_x += position.x as f32 / 100.0;
                    self.wheel_y += position.y as f32 / 100.0;
                    let x = whole_steps(&mut self.wheel_x);
                    let y = whole_steps(&mut self.wheel_y);
                    (x != 0.0 || y != 0.0).then_some(InputEvent::WheelSteps { x, y })
                }
            }
        }
    }
    struct App {
        canvas: HtmlCanvasElement,
        window: Rc<RefCell<Option<Arc<Window>>>>,
        renderer: Rc<RefCell<Option<Renderer>>>,
        engine: Rc<RefCell<Option<Engine>>>,
        storage: Rc<RefCell<Option<Rc<crate::storage::IndexedDbStorage>>>>,
        video: Rc<RefCell<Option<Rc<RefCell<crate::video::WebVideoBackend>>>>>,
        last_tick: web_time::Instant,
        input: BrowserInput,
        audio_state: JsValue,
        focused: bool,
        touch: Option<TouchTrack>,
    }
    impl App {
        fn forward_pointer_moved(&mut self, px: f32, py: f32) {
            self.input.cursor_physical = Some((px, py));
            if let (Some(engine), Some(window)) = (
                self.engine.borrow_mut().as_mut(),
                self.window.borrow().as_ref(),
            ) {
                let size = window.inner_size();
                let (x, y) = Viewport::new(size.width, size.height).to_logical(px, py);
                engine.input(InputEvent::PointerMoved { x, y });
            }
        }
        fn feed_input(&mut self, input: InputEvent) {
            if let Some(engine) = self.engine.borrow_mut().as_mut() {
                engine.input(input);
            }
        }
        fn synthesize_click(&mut self, button: MouseButton, px: f32, py: f32) {
            self.input.cursor_physical = Some((px, py));
            let press = self.input.pointer_button(button, ElementState::Pressed);
            let release = self.input.pointer_button(button, ElementState::Released);
            self.feed_input(press);
            self.feed_input(release);
        }
    }
    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.borrow().is_some() {
                return;
            }
            let dpr = web_sys::window()
                .map_or(1.0, |window| window.device_pixel_ratio());
            let width = (f64::from(self.canvas.client_width().max(1)) * dpr)
                .round()
                .clamp(1.0, f64::from(u32::MAX)) as u32;
            let height = (f64::from(self.canvas.client_height().max(1)) * dpr)
                .round()
                .clamp(1.0, f64::from(u32::MAX)) as u32;
            self.canvas.set_width(width);
            self.canvas.set_height(height);
            let attributes = Window::default_attributes()
                .with_title("eden Web")
                .with_inner_size(PhysicalSize::new(width, height))
                .with_canvas(Some(self.canvas.clone()))
                .with_prevent_default(true)
                .with_focusable(true);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    set_status(true, &format!("创建 Web Canvas 失败：{error}"));
                    return;
                }
            };
            window.set_prevent_default(true);
            let renderer_slot = Rc::clone(&self.renderer);
            let engine_slot = Rc::clone(&self.engine);
            let storage_slot = Rc::clone(&self.storage);
            let render_window = Arc::clone(&window);
            spawn_local(async move {
                match Renderer::new(Arc::clone(&render_window), width, height).await {
                    Ok(mut renderer) => {
                        if let Err(error) = renderer.render_clear(SMOKE_COLOR) {
                            set_status(
                                true,
                                &format!("WebGPU 测试帧失败：{error:#}"),
                            );
                            return;
                        }
                        let storage = match crate::storage::initialize().await {
                            Ok(storage) => storage,
                            Err(error) => {
                                set_status(
                                    true,
                                    &format!("IndexedDB 初始化失败：{error}"),
                                );
                                return;
                            }
                        };
                        let restore = crate::browser_fs::restore_saved_mount().await;
                        *renderer_slot.borrow_mut() = Some(renderer);
                        *storage_slot.borrow_mut() = Some(Rc::clone(&storage));
                        if let Some(canvas) = render_window.canvas() {
                            let _ = canvas.set_attribute("data-wasm-ready", "true");
                        }
                        match restore {
                            Ok(
                                crate::browser_fs::RestoreResult::NeedsPermission(name),
                            ) => {
                                set_status(
                                    false,
                                    &format!(
                                        "已记录游戏目录 {name}；请点击“选择游戏目录”重新选择。"
                                    ),
                                );
                            }
                            Ok(crate::browser_fs::RestoreResult::None) => {
                                set_status(
                                    false,
                                    "WASM、WebGPU 与 IndexedDB 已就绪。",
                                );
                            }
                            Err(error) => {
                                set_status(
                                    true,
                                    &format!("恢复游戏目录失败：{error}"),
                                );
                            }
                        }
                        set_directory_button_enabled(engine_slot.borrow().is_none());
                        set_button_enabled("clear-web-data", true);
                        set_button_enabled("import-save", true);
                        set_button_enabled("export-saves", true);
                        render_window.request_redraw();
                    }
                    Err(error) => {
                        set_status(true, &format!("WebGPU 初始化失败：{error:#}"));
                    }
                }
            });
            *self.window.borrow_mut() = Some(window);
        }
        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        engine.resize(size.width, size.height);
                    } else if let Some(renderer) = self.renderer.borrow_mut().as_mut() {
                        renderer.resize(size.width, size.height);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.input.cursor_physical = Some((
                        position.x as f32,
                        position.y as f32,
                    ));
                    if let (Some(engine), Some(window)) = (
                        self.engine.borrow_mut().as_mut(),
                        self.window.borrow().as_ref(),
                    ) {
                        let size = window.inner_size();
                        let (x, y) = Viewport::new(size.width, size.height)
                            .to_logical(position.x as f32, position.y as f32);
                        engine.input(InputEvent::PointerMoved { x, y });
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    self.input.cursor_physical = None;
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        engine.input(InputEvent::PointerLeft);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    if state == ElementState::Pressed {
                        crate::audio::resume(&self.audio_state);
                    }
                    let input = self.input.pointer_button(button, state);
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        engine.input(input);
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    if let Some(input) = self.input.wheel(delta) {
                        if let Some(engine) = self.engine.borrow_mut().as_mut() {
                            engine.input(input);
                        }
                    }
                }
                WindowEvent::Touch(touch) => {
                    let (px, py) = (touch.location.x as f32, touch.location.y as f32);
                    let tracked = self
                        .touch
                        .as_ref()
                        .is_some_and(|track| track.finger == touch.id);
                    match touch.phase {
                        TouchPhase::Started => {
                            if self.touch.is_none() {
                                crate::audio::resume(&self.audio_state);
                                self.touch = Some(TouchTrack {
                                    finger: touch.id,
                                    start: (px, py),
                                    last: (px, py),
                                    started_at: web_time::Instant::now(),
                                    consumed: false,
                                });
                                self.forward_pointer_moved(px, py);
                            }
                        }
                        TouchPhase::Moved => {
                            if let Some(track) = self.touch.as_mut() {
                                track.last = (px, py);
                            }
                            if tracked {
                                self.forward_pointer_moved(px, py);
                            }
                        }
                        TouchPhase::Ended => {
                            if tracked {
                                let tap = self
                                    .touch
                                    .take()
                                    .is_some_and(|track| {
                                        track.is_tap(web_time::Instant::now())
                                    });
                                if tap {
                                    self.synthesize_click(MouseButton::Left, px, py);
                                }
                            }
                        }
                        TouchPhase::Cancelled => {
                            self.touch = None;
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        crate::audio::resume(&self.audio_state);
                    }
                    if let Some(key) = input_key(&event.logical_key) {
                        if let Some(engine) = self.engine.borrow_mut().as_mut() {
                            engine
                                .input(InputEvent::Key {
                                    key,
                                    state: button_state(event.state),
                                    repeat: event.repeat,
                                });
                        }
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused {
                        self.input.cursor_physical = None;
                        self.input.last_press = None;
                        self.touch = None;
                    }
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        if focused {
                            engine.notify_focus_gained();
                        } else {
                            engine.notify_focus_lost();
                            engine.input(InputEvent::PointerLeft);
                            engine
                                .input(InputEvent::Key {
                                    key: InputKey::Control,
                                    state: ButtonState::Released,
                                    repeat: false,
                                });
                        }
                    }
                    let background = self
                        .engine
                        .borrow()
                        .as_ref()
                        .map(|engine| engine.settings.borrow().progress_in_background)
                        .unwrap_or(0);
                    if let Some(video) = self.video.borrow().as_ref() {
                        video.borrow().set_blur_paused(!focused && background == 0);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Some(engine) = self.engine.borrow_mut().as_mut() {
                        engine.render();
                    } else if let Some(renderer) = self.renderer.borrow_mut().as_mut() {
                        if let Err(error) = renderer.render_clear(SMOKE_COLOR) {
                            set_status(
                                true,
                                &format!("WebGPU 呈现失败：{error:#}"),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self
                .touch
                .as_ref()
                .is_some_and(|track| track.long_press_due(web_time::Instant::now()))
            {
                if let Some(track) = self.touch.as_mut() {
                    track.consumed = true;
                }
                if let Some((px, py)) = self.touch.as_ref().map(|track| track.last) {
                    self.synthesize_click(MouseButton::Right, px, py);
                }
            }
            if let Some(engine) = self.engine.borrow_mut().as_mut() {
                let now = web_time::Instant::now();
                let delta = now.saturating_duration_since(self.last_tick);
                self.last_tick = now;
                let paused = !self.focused
                    && engine.settings.borrow().progress_in_background == 0;
                if !paused {
                    engine
                        .advance_time(delta.min(std::time::Duration::from_millis(250)));
                }
                if let Some(window) = self.window.borrow().as_ref() {
                    window.request_redraw();
                }
                if engine.quit {
                    event_loop.exit();
                }
                event_loop
                    .set_control_flow(
                        if paused { ControlFlow::Wait } else { ControlFlow::Poll },
                    );
            } else {
                self.last_tick = web_time::Instant::now();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            if self.engine.borrow().is_some() {
                dispatch_platform_requests(&self.engine, &self.storage, &self.canvas);
            }
        }
    }
    fn dispatch_platform_requests(
        engine_slot: &Rc<RefCell<Option<Engine>>>,
        storage_slot: &Rc<RefCell<Option<Rc<crate::storage::IndexedDbStorage>>>>,
        canvas: &HtmlCanvasElement,
    ) {
        let requests = engine_slot
            .borrow_mut()
            .as_mut()
            .map(Engine::drain_platform_requests)
            .unwrap_or_default();
        for request in requests {
            match request {
                PlatformRequest::SetScreenMode { mode } => {
                    apply_fullscreen(canvas, mode.is_fullscreen());
                    if let Some(engine) = engine_slot.borrow_mut().as_mut() {
                        engine
                            .platform_response(PlatformResponse::ScreenModeApplied {
                                mode,
                                ok: true,
                            });
                    }
                }
                PlatformRequest::SetPointerCapture { .. } => {}
                PlatformRequest::RestoreWindowState { .. } => {}
                PlatformRequest::QueryWindowGeometry => {
                    let dpr = web_sys::window()
                        .map_or(1.0, |window| window.device_pixel_ratio());
                    let width = (f64::from(canvas.client_width().max(1)) * dpr)
                        .round()
                        .clamp(1.0, f64::from(u32::MAX)) as u32;
                    let height = (f64::from(canvas.client_height().max(1)) * dpr)
                        .round()
                        .clamp(1.0, f64::from(u32::MAX)) as u32;
                    if let Some(engine) = engine_slot.borrow_mut().as_mut() {
                        engine
                            .platform_response(
                                PlatformResponse::WindowGeometry(eden_engine::WindowGeometry {
                                    x: 0,
                                    y: 0,
                                    width,
                                    height,
                                    maximized: false,
                                }),
                            );
                    }
                }
                PlatformRequest::ShowContextMenu { request_id, state } => {
                    let object = Object::new();
                    for (name, value) in [
                        ("panelHidden", state.panel_hidden),
                        ("panelToggleEnabled", state.panel_toggle_enabled),
                        ("autoChecked", state.auto_checked),
                        ("skipChecked", state.skip_checked),
                        ("saveLoadEnabled", state.save_load_enabled),
                        ("fullscreen", state.screen_mode.is_fullscreen()),
                    ] {
                        let _ = Reflect::set(
                            &object,
                            &JsValue::from_str(name),
                            &JsValue::from_bool(value),
                        );
                    }
                    let engine_slot = Rc::clone(engine_slot);
                    let canvas = canvas.clone();
                    spawn_local(async move {
                        let value = JsFuture::from(show_context_menu(object.as_ref()))
                            .await
                            .ok()
                            .and_then(|value| value.as_string());
                        if value.as_deref() == Some("fullscreen") {
                            apply_fullscreen(&canvas, true);
                        } else if value.as_deref() == Some("windowed") {
                            apply_fullscreen(&canvas, false);
                        }
                        let command = value.and_then(|value| context_command(&value));
                        if let Some(engine) = engine_slot.borrow_mut().as_mut() {
                            engine
                                .platform_response(PlatformResponse::ContextMenuClosed {
                                    request_id,
                                    command,
                                });
                        }
                    });
                }
                PlatformRequest::EditSaveComment { request_id, initial, max_chars } => {
                    let engine_slot = Rc::clone(engine_slot);
                    spawn_local(async move {
                        let comment = JsFuture::from(
                                edit_save_comment(&initial, max_chars),
                            )
                            .await
                            .ok()
                            .and_then(|value| value.as_string());
                        if let Some(engine) = engine_slot.borrow_mut().as_mut() {
                            engine
                                .platform_response(PlatformResponse::SaveCommentClosed {
                                    request_id,
                                    comment,
                                });
                        }
                    });
                }
                PlatformRequest::PersistSave { save_name, bytes } => {
                    let Some(storage) = storage_slot.borrow().clone() else {
                        set_status(true, "IndexedDB 尚未就绪，无法保存。");
                        continue;
                    };
                    let engine_slot = Rc::clone(engine_slot);
                    let canvas = canvas.clone();
                    spawn_local(async move {
                        set_status(false, "正在保存…");
                        let placeholder = match crate::transfer::placeholder_thumbnail_png() {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                set_status(
                                    true,
                                    &format!("生成存档占位图失败：{error}"),
                                );
                                return;
                            }
                        };
                        if let Err(error) = storage
                            .write_save_pair(&save_name, &bytes, &placeholder)
                            .await
                        {
                            set_status(
                                true,
                                &format!("存档写入 IndexedDB 失败：{error}"),
                            );
                            return;
                        }
                        let thumbnail_name = format!(
                            "{}.png", & save_name[..save_name.len() - 4]
                        );
                        if let Some(engine) = engine_slot.borrow().as_ref() {
                            engine
                                .invalidate_resource_image(
                                    &format!("~save/{thumbnail_name}"),
                                );
                        }
                        set_status(false, "存档已保存，正在生成缩略图…");
                        match JsFuture::from(capture_thumbnail(&canvas)).await {
                            Ok(value) => {
                                let array = Uint8Array::new(&value);
                                let mut png = vec![0; array.length() as usize];
                                array.copy_to(&mut png);
                                if let Err(error) = storage
                                    .write_durable(&format!("saves/{thumbnail_name}"), &png)
                                    .await
                                {
                                    log::warn!(
                                        "真实存档缩略图写入失败，保留占位图: {error}"
                                    );
                                    set_status(
                                        false,
                                        "存档已保存（缩略图使用占位图）。",
                                    );
                                    return;
                                }
                                if let Some(engine) = engine_slot.borrow().as_ref() {
                                    engine
                                        .invalidate_resource_image(
                                            &format!("~save/{thumbnail_name}"),
                                        );
                                }
                                set_status(false, "存档已保存。");
                            }
                            Err(error) => {
                                log::warn!(
                                    "Canvas 缩略图生成失败，保留占位图: {}",
                                    js_value(error)
                                );
                                set_status(
                                    false,
                                    "存档已保存（缩略图使用占位图）。",
                                );
                            }
                        }
                    });
                }
                PlatformRequest::OpenHelp => {
                    set_status(false, "Web 开发版暂未打包原版帮助文件。");
                }
                PlatformRequest::OpenUrl { url } => {
                    if let Some(window) = web_sys::window() {
                        let _ = window.open_with_url_and_target(&url, "_blank");
                    }
                }
            }
        }
    }
    fn context_command(value: &str) -> Option<ContextMenuCommand> {
        Some(
            match value {
                "title" => ContextMenuCommand::ReturnToTitle,
                "toggle-panel" => ContextMenuCommand::ToggleMessagePanel,
                "auto" => ContextMenuCommand::ToggleAuto,
                "skip" => ContextMenuCommand::ToggleSkip,
                "quick-save" => ContextMenuCommand::QuickSave,
                "save" => ContextMenuCommand::Save,
                "load" => ContextMenuCommand::Load,
                "config" => ContextMenuCommand::Config,
                "windowed" => ContextMenuCommand::ScreenWindowed,
                "fullscreen" => ContextMenuCommand::ScreenBorderless,
                "help" => ContextMenuCommand::Help,
                "website" => ContextMenuCommand::Website,
                "exit" => ContextMenuCommand::Exit,
                _ => return None,
            },
        )
    }
    fn js_value(value: JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                Reflect::get(&value, &JsValue::from_str("message")).ok()?.as_string()
            })
            .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
            .unwrap_or_else(|| "未知浏览器错误".into())
    }
    fn whole_steps(value: &mut f32) -> f32 {
        let steps = if *value >= 0.0 { value.floor() } else { value.ceil() };
        *value -= steps;
        steps
    }
    fn button_state(state: ElementState) -> ButtonState {
        match state {
            ElementState::Pressed => ButtonState::Pressed,
            ElementState::Released => ButtonState::Released,
        }
    }
    fn input_key(key: &Key) -> Option<InputKey> {
        Some(
            match key {
                Key::Named(named) => {
                    match named {
                        NamedKey::Enter => InputKey::Enter,
                        NamedKey::Space => InputKey::Space,
                        NamedKey::Escape => InputKey::Escape,
                        NamedKey::ArrowUp => InputKey::ArrowUp,
                        NamedKey::ArrowDown => InputKey::ArrowDown,
                        NamedKey::ArrowLeft => InputKey::ArrowLeft,
                        NamedKey::ArrowRight => InputKey::ArrowRight,
                        NamedKey::PageUp => InputKey::PageUp,
                        NamedKey::PageDown => InputKey::PageDown,
                        NamedKey::Home => InputKey::Home,
                        NamedKey::End => InputKey::End,
                        NamedKey::Control => InputKey::Control,
                        NamedKey::F5 => InputKey::F5,
                        NamedKey::F9 => InputKey::F9,
                        _ => InputKey::Other,
                    }
                }
                Key::Character(value) => InputKey::Character(value.to_string()),
                _ => InputKey::Other,
            },
        )
    }
    fn start_engine(
        renderer_slot: &Rc<RefCell<Option<Renderer>>>,
        engine_slot: &Rc<RefCell<Option<Engine>>>,
        storage: Rc<crate::storage::IndexedDbStorage>,
        window: Arc<Window>,
        video_slot: &Rc<RefCell<Option<Rc<RefCell<crate::video::WebVideoBackend>>>>>,
        audio_state: JsValue,
    ) -> Result<(), String> {
        if engine_slot.borrow().is_some() {
            return Ok(());
        }
        let redraw_window = Arc::clone(&window);
        let resource_storage: Rc<dyn StorageBackend> = storage.clone();
        let resources = crate::browser_fs::create_resource_store(
            Rc::new(move || {
                redraw_window.request_redraw();
            }),
            resource_storage,
        )?;
        let (font_bytes, font_name) = crate::browser_fs::mounted_font()?;
        eden_engine::GlyphAtlas::from_bytes_named(font_bytes.clone(), font_name.clone())
            .map_err(|error| format!("Web 字体无效: {error}"))?;
        let progress = storage
            .read("system.dat")
            .ok()
            .flatten()
            .and_then(|bytes| eden_formats::system_dat::decode(&bytes).ok())
            .unwrap_or_else(BTreeMap::new);
        let settings = storage
            .read("settings.tsv")
            .ok()
            .flatten()
            .map_or_else(Settings::default, |bytes| Settings::from_bytes(&bytes));
        let cg_seen = storage
            .read("cgseen.tsv")
            .ok()
            .flatten()
            .map_or_else(CgSeen::default, |bytes| CgSeen::from_bytes(&bytes));
        let read_history = storage
            .read("readflags.bin")
            .ok()
            .flatten()
            .map_or_else(
                eden_engine::read_history::ReadHistory::default,
                |bytes| { eden_engine::read_history::ReadHistory::from_bytes(&bytes) },
            );
        let renderer = renderer_slot
            .borrow_mut()
            .take()
            .ok_or_else(|| "Renderer 尚未就绪或已被消费".to_string())?;
        let video = Rc::new(
            RefCell::new(
                crate::video::WebVideoBackend::new(
                    renderer.device.clone(),
                    renderer.queue.clone(),
                ),
            ),
        );
        let video_backend: Rc<RefCell<dyn VideoBackend>> = video.clone();
        let storage: Rc<dyn StorageBackend> = storage;
        let engine = Engine::assemble(
                renderer,
                EngineServices {
                    resources: Rc::clone(&resources),
                    native_vfs: None,
                    audio: Rc::new(
                        RefCell::new(
                            crate::audio::WebAudioBackend::new(audio_state, resources),
                        ),
                    ),
                    storage,
                    video: video_backend,
                    font: FontSource::bytes(font_bytes, font_name),
                    settings: Rc::new(RefCell::new(settings)),
                    cg_seen: Rc::new(RefCell::new(cg_seen)),
                    read_history: Rc::new(RefCell::new(read_history)),
                    progress,
                },
            )
            .map_err(|error| error.to_string())?;
        *engine_slot.borrow_mut() = Some(engine);
        *video_slot.borrow_mut() = Some(video);
        window.request_redraw();
        Ok(())
    }
    fn set_status(error: bool, message: &str) {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        if let Some(status) = document.get_element_by_id("status") {
            status
                .set_class_name(
                    if error { "status status--error" } else { "status status--ready" },
                );
            status.set_text_content(Some(message));
        }
    }
    fn show_mount_summary(summary: &crate::browser_fs::MountSummary) {
        let warning = if summary.warnings.is_empty() {
            String::new()
        } else {
            format!("；警告：{}", summary.warnings.join("；"))
        };
        set_status(
            false,
            &format!(
                "目录 {} 已挂载：{} 个归档 / {} 个条目 / {} 个脚本 / {} 个 override，字体={}{}",
                summary.directory_name, summary.archive_count, summary.archive_entries,
                summary.script_count, summary.override_count, if summary.has_font {
                "已加载" } else { "缺失" }, warning
            ),
        );
    }
    fn set_directory_button_enabled(enabled: bool) {
        set_button_enabled("select-directory", enabled);
    }
    fn set_button_enabled(id: &str, enabled: bool) {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        if let Some(button) = document
            .get_element_by_id(id)
            .and_then(|element| element.dyn_into::<HtmlButtonElement>().ok())
        {
            button.set_disabled(!enabled);
        }
    }
    fn install_directory_picker(
        document: &web_sys::Document,
        renderer_slot: Rc<RefCell<Option<Renderer>>>,
        engine_slot: Rc<RefCell<Option<Engine>>>,
        storage_slot: Rc<RefCell<Option<Rc<crate::storage::IndexedDbStorage>>>>,
        window_slot: Rc<RefCell<Option<Arc<Window>>>>,
        video_slot: Rc<RefCell<Option<Rc<RefCell<crate::video::WebVideoBackend>>>>>,
        audio_state: JsValue,
    ) -> Result<(), JsValue> {
        let button = document
            .get_element_by_id("select-directory")
            .ok_or_else(|| JsValue::from_str("#select-directory is missing"))?
            .dyn_into::<HtmlButtonElement>()?;
        let callback_button = button.clone();
        let callback = Closure::<
            dyn FnMut(Event),
        >::new(move |_event| {
            crate::audio::resume(&audio_state);
            callback_button.set_disabled(true);
            set_status(
                false,
                "正在验证目录、加载 PAZ 索引与全部脚本…",
            );
            let done_button = callback_button.clone();
            let renderer_slot = Rc::clone(&renderer_slot);
            let engine_slot = Rc::clone(&engine_slot);
            let storage_slot = Rc::clone(&storage_slot);
            let window_slot = Rc::clone(&window_slot);
            let video_slot = Rc::clone(&video_slot);
            let audio_state = audio_state.clone();
            spawn_local(async move {
                match crate::browser_fs::select_and_mount().await {
                    Ok(summary) => {
                        let storage = storage_slot.borrow().clone();
                        let window = window_slot.borrow().clone();
                        match (storage, window) {
                            (Some(storage), Some(window)) => {
                                match start_engine(
                                    &renderer_slot,
                                    &engine_slot,
                                    storage,
                                    window,
                                    &video_slot,
                                    audio_state,
                                ) {
                                    Ok(()) => show_mount_summary(&summary),
                                    Err(error) => {
                                        set_status(true, &format!("Engine 组装失败：{error}"));
                                        done_button.set_disabled(false);
                                    }
                                }
                            }
                            _ => {
                                set_status(
                                    true,
                                    "WebGPU 或 IndexedDB 尚未就绪，请稍后重试。",
                                );
                                done_button.set_disabled(false);
                            }
                        }
                    }
                    Err(error) => {
                        if error == crate::browser_fs::PICKER_CANCELLED {
                            set_status(
                                false,
                                "已取消目录选择，可重新点击“选择游戏目录”。",
                            );
                        } else {
                            set_status(true, &format!("目录挂载失败：{error}"));
                        }
                        done_button.set_disabled(false);
                    }
                }
            });
        });
        button
            .add_event_listener_with_callback(
                "click",
                callback.as_ref().unchecked_ref(),
            )?;
        callback.forget();
        Ok(())
    }
    fn install_save_transfer_buttons(
        document: &web_sys::Document,
        storage_slot: Rc<RefCell<Option<Rc<crate::storage::IndexedDbStorage>>>>,
        engine_slot: Rc<RefCell<Option<Engine>>>,
    ) -> Result<(), JsValue> {
        let import_button = document
            .get_element_by_id("import-save")
            .ok_or_else(|| JsValue::from_str("#import-save is missing"))?
            .dyn_into::<HtmlButtonElement>()?;
        let import_callback_button = import_button.clone();
        let import_storage = Rc::clone(&storage_slot);
        let import_engine = Rc::clone(&engine_slot);
        let import_callback = Closure::<
            dyn FnMut(Event),
        >::new(move |_event| {
            import_callback_button.set_disabled(true);
            set_status(false, "请选择一个 .sav 存档文件…");
            let done_button = import_callback_button.clone();
            let storage_slot = Rc::clone(&import_storage);
            let engine_slot = Rc::clone(&import_engine);
            spawn_local(async move {
                let picked = match JsFuture::from(pick_save_file()).await {
                    Ok(value) => value,
                    Err(error) => {
                        if js_error_name(&error).as_deref() == Some("AbortError") {
                            set_status(false, "已取消导入。");
                        } else {
                            set_status(
                                true,
                                &format!("选择存档失败：{}", js_value(error)),
                            );
                        }
                        done_button.set_disabled(false);
                        return;
                    }
                };
                let file_name = match Reflect::get(&picked, &JsValue::from_str("name"))
                    .ok()
                    .and_then(|value| value.as_string())
                {
                    Some(name) => name,
                    None => {
                        set_status(
                            true,
                            "文件选择器没有返回有效文件名。",
                        );
                        done_button.set_disabled(false);
                        return;
                    }
                };
                let value = match Reflect::get(&picked, &JsValue::from_str("bytes")) {
                    Ok(value) => value,
                    Err(error) => {
                        set_status(
                            true,
                            &format!("读取存档失败：{}", js_value(error)),
                        );
                        done_button.set_disabled(false);
                        return;
                    }
                };
                let array = Uint8Array::new(&value);
                let mut bytes = vec![0; array.length() as usize];
                array.copy_to(&mut bytes);
                let comment = match crate::transfer::validate_save(&bytes) {
                    Ok(comment) => comment,
                    Err(error) => {
                        set_status(true, &error);
                        done_button.set_disabled(false);
                        return;
                    }
                };
                let suggested = crate::transfer::suggested_slot(&file_name);
                let selected = match JsFuture::from(
                        choose_import_slot(&file_name, suggested),
                    )
                    .await
                {
                    Ok(value) if !value.is_null() && !value.is_undefined() => {
                        value.as_f64().map(|value| value as usize)
                    }
                    _ => None,
                };
                let Some(slot) = selected.filter(|slot| (10..=99).contains(slot)) else {
                    set_status(false, "已取消导入。");
                    done_button.set_disabled(false);
                    return;
                };
                let Some(storage) = storage_slot.borrow().clone() else {
                    set_status(true, "IndexedDB 尚未就绪，无法导入。");
                    done_button.set_disabled(false);
                    return;
                };
                let save_name = format!("eden{slot:04}.sav");
                if storage.has_save(&save_name) && !confirm_save_overwrite(slot) {
                    set_status(false, "已取消覆盖。");
                    done_button.set_disabled(false);
                    return;
                }
                let thumbnail = match crate::transfer::placeholder_thumbnail_png() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        set_status(
                            true,
                            &format!("生成占位缩略图失败：{error}"),
                        );
                        done_button.set_disabled(false);
                        return;
                    }
                };
                match storage.write_save_pair(&save_name, &bytes, &thumbnail).await {
                    Ok(()) => {
                        let thumbnail_name = format!("eden{slot:04}.png");
                        if let Some(engine) = engine_slot.borrow().as_ref() {
                            engine
                                .invalidate_resource_image(
                                    &format!("~save/{thumbnail_name}"),
                                );
                        }
                        let suffix = if comment.is_empty() {
                            String::new()
                        } else {
                            format!("：{comment}")
                        };
                        set_status(false, &format!("已导入到槽位 {slot}{suffix}"));
                    }
                    Err(error) => {
                        set_status(true, &format!("导入存档写入失败：{error}"));
                    }
                }
                done_button.set_disabled(false);
            });
        });
        import_button
            .add_event_listener_with_callback(
                "click",
                import_callback.as_ref().unchecked_ref(),
            )?;
        import_callback.forget();
        let export_button = document
            .get_element_by_id("export-saves")
            .ok_or_else(|| JsValue::from_str("#export-saves is missing"))?
            .dyn_into::<HtmlButtonElement>()?;
        let export_callback_button = export_button.clone();
        let export_callback = Closure::<
            dyn FnMut(Event),
        >::new(move |_event| {
            export_callback_button.set_disabled(true);
            let Some(storage) = storage_slot.borrow().clone() else {
                set_status(true, "IndexedDB 尚未就绪，无法导出。");
                export_callback_button.set_disabled(false);
                return;
            };
            match crate::transfer::build_saves_zip(storage.save_entries()) {
                Ok(bytes) => {
                    let value: JsValue = Uint8Array::from(bytes.as_slice()).into();
                    download_bytes("eden-web-saves.zip", "application/zip", &value);
                    set_status(false, "存档 ZIP 已导出。");
                }
                Err(error) => set_status(true, &format!("导出失败：{error}")),
            }
            export_callback_button.set_disabled(false);
        });
        export_button
            .add_event_listener_with_callback(
                "click",
                export_callback.as_ref().unchecked_ref(),
            )?;
        export_callback.forget();
        Ok(())
    }
    fn install_clear_data_button(document: &web_sys::Document) -> Result<(), JsValue> {
        let button = document
            .get_element_by_id("clear-web-data")
            .ok_or_else(|| JsValue::from_str("#clear-web-data is missing"))?
            .dyn_into::<HtmlButtonElement>()?;
        let callback_button = button.clone();
        let callback = Closure::<
            dyn FnMut(Event),
        >::new(move |_event| {
            callback_button.set_disabled(true);
            let done_button = callback_button.clone();
            spawn_local(async move {
                match crate::storage::clear_active().await {
                    Ok(()) => {
                        set_status(
                            false,
                            "当前 origin 的 eden IndexedDB 已清除。",
                        )
                    }
                    Err(error) => {
                        set_status(true, &format!("清除 IndexedDB 失败：{error}"));
                    }
                }
                done_button.set_disabled(false);
            });
        });
        button
            .add_event_listener_with_callback(
                "click",
                callback.as_ref().unchecked_ref(),
            )?;
        callback.forget();
        Ok(())
    }
    fn js_error(error: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&error.to_string())
    }
    fn js_error_name(value: &JsValue) -> Option<String> {
        Reflect::get(value, &JsValue::from_str("name"))
            .ok()
            .and_then(|name| name.as_string())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        console_error_panic_hook::set_once();
        let _ = log::set_logger(&BROWSER_LOGGER);
        log::set_max_level(log::LevelFilter::Debug);
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("document unavailable"))?;
        let canvas = document
            .get_element_by_id("eden-canvas")
            .ok_or_else(|| JsValue::from_str("#eden-canvas is missing"))?
            .dyn_into::<HtmlCanvasElement>()?;
        let window_slot = Rc::new(RefCell::new(None));
        let renderer_slot = Rc::new(RefCell::new(None));
        let engine_slot = Rc::new(RefCell::new(None));
        let storage_slot = Rc::new(RefCell::new(None));
        let video_slot: Rc<
            RefCell<Option<Rc<RefCell<crate::video::WebVideoBackend>>>>,
        > = Rc::new(RefCell::new(None));
        let audio_state = crate::audio::create_state()
            .map_err(|error| JsValue::from_str(
                &format!("WebAudio 初始化失败：{error}"),
            ))?;
        install_directory_picker(
            &document,
            Rc::clone(&renderer_slot),
            Rc::clone(&engine_slot),
            Rc::clone(&storage_slot),
            Rc::clone(&window_slot),
            Rc::clone(&video_slot),
            audio_state.clone(),
        )?;
        install_save_transfer_buttons(
            &document,
            Rc::clone(&storage_slot),
            Rc::clone(&engine_slot),
        )?;
        install_clear_data_button(&document)?;
        let event_loop = EventLoop::new().map_err(js_error)?;
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop
            .spawn_app(App {
                canvas,
                window: window_slot,
                renderer: renderer_slot,
                engine: engine_slot,
                storage: storage_slot,
                video: video_slot,
                last_tick: web_time::Instant::now(),
                input: BrowserInput::default(),
                audio_state,
                focused: true,
                touch: None,
            });
        Ok(())
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn browser_entry_available() -> bool {
    false
}

