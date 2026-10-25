#![allow(non_snake_case)]
//! ShinkiEMU C ABI v3 adapter around the original ReMinori Engine.
mod config;
mod platform;
use shinki_emu_abi as abi;
use eden_engine::{ButtonState, Engine, InputEvent, InputKey, PointerButton};
use eden_vm::RunState;
use std::{
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn bytes(text: &'static str) -> abi::Bytes {
    abi::Bytes::from_static(text.as_bytes())
}
#[derive(Debug)]
struct Failure {
    status: abi::Status,
    code: &'static str,
    message: String,
}
impl Failure {
    fn invalid(message: &str) -> Self {
        Self {
            status: abi::INVALID_ARGUMENT,
            code: "REMINORI_ARGUMENT",
            message: message.into(),
        }
    }
    fn status(status: abi::Status) -> Self {
        Self {
            status,
            code: "REMINORI_ABI",
            message: format!("ABI request failed with status {status}"),
        }
    }
    fn runtime(error: impl std::fmt::Display) -> Self {
        Self {
            status: abi::CORE_ERROR,
            code: "REMINORI_NATIVE",
            message: error.to_string(),
        }
    }
    fn panic() -> Self {
        Self {
            status: abi::PANICKED,
            code: "REMINORI_PANIC",
            message: "native engine panicked; close the session".into(),
        }
    }
    fn error(&self) -> abi::Error {
        let mut error = abi::Error::default();
        error.set(self.code, &self.message);
        error
    }
}
fn boundary(error: *mut abi::Error, body: impl FnOnce() -> Result<(), Failure>) -> abi::Status {
    if !error.is_null() {
        unsafe {
            error.write(abi::Error::default());
        }
    }
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => abi::OK,
        result => {
            let failure = match result {
                Ok(Err(error)) => error,
                _ => Failure::panic(),
            };
            if !error.is_null() {
                unsafe {
                    error.write(failure.error());
                }
            }
            failure.status
        }
    }
}
#[derive(Clone)]
struct Host {
    services: abi::HostServices,
    epoch: u64,
    local_cancel: Arc<AtomicBool>,
}
// ABI v3 services are thread-safe and host-owned through failed open/close.
unsafe impl Send for Host {}
unsafe impl Sync for Host {}
impl Host {
    fn cancelled(&self) -> bool {
        self.local_cancel.load(Ordering::Acquire)
            || self
                .services
                .cancellation
                .is_cancelled
                .is_some_and(|f| unsafe { f(self.services.cancellation.context, self.epoch) != 0 })
    }
    fn cancel(&self) {
        self.local_cancel.store(true, Ordering::Release);
        if let Some(invalidate) = self.services.window.invalidate {
            unsafe {
                invalidate(self.services.window.context, self.epoch, 1);
            }
        }
    }
}
struct DiagnosticSink(abi::Diagnostics);
unsafe impl Send for DiagnosticSink {}
unsafe impl Sync for DiagnosticSink {}
static DIAGNOSTICS: OnceLock<DiagnosticSink> = OnceLock::new();
struct Logger;
static LOGGER: Logger = Logger;
impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Some(sink) = DIAGNOSTICS.get() else {
            return;
        };
        let Some(emit) = sink.0.emit else {
            return;
        };
        let message = record.args().to_string();
        let record = abi::LogRecord {
            level: match record.level() {
                log::Level::Error => abi::LOG_ERROR,
                log::Level::Warn => abi::LOG_WARN,
                log::Level::Info => abi::LOG_INFO,
                log::Level::Debug => abi::LOG_DEBUG,
                log::Level::Trace => abi::LOG_TRACE,
            },
            dropped_fields: 0,
            target: abi::Bytes::new(record.target().as_bytes()),
            event: abi::Bytes::EMPTY,
            message: abi::Bytes::new(message.as_bytes()),
            fields: std::ptr::null(),
            field_count: 0,
        };
        unsafe {
            emit(sink.0.context, &record);
        }
    }
    fn flush(&self) {
        if let Some(sink) = DIAGNOSTICS.get() {
            if let Some(flush) = sink.0.flush {
                unsafe {
                    flush(sink.0.context);
                }
            }
        }
    }
}

fn numeric_field(name: &'static str, value: u64) -> abi::LogField {
    abi::LogField {
        name: bytes(name),
        kind: abi::FIELD_U64,
        reserved: 0,
        text: abi::Bytes::EMPTY,
        signed: 0,
        unsigned: value,
        number: 0.0,
    }
}
fn emit_event(level: u32, event: &'static str, fields: &[abi::LogField]) {
    let Some(sink) = DIAGNOSTICS.get() else {
        return;
    };
    let Some(emit) = sink.0.emit else {
        return;
    };
    let record = abi::LogRecord {
        level,
        dropped_fields: 0,
        target: bytes("ReMinori"),
        event: bytes(event),
        message: abi::Bytes::EMPTY,
        fields: fields.as_ptr(),
        field_count: fields.len(),
    };
    unsafe {
        emit(sink.0.context, &record);
    }
}
fn platform_warning(operation: u64, failure: &Failure) {
    let mut status = numeric_field("status", 0);
    status.kind = abi::FIELD_I64;
    status.signed = i64::from(failure.status);
    let text_field = |name, text: &str| abi::LogField {
        name: bytes(name),
        kind: abi::FIELD_TEXT,
        reserved: 0,
        text: abi::Bytes::new(text.as_bytes()),
        signed: 0,
        unsigned: 0,
        number: 0.0,
    };
    emit_event(
        abi::LOG_WARN,
        "reminori.platform_warning",
        &[
            numeric_field("operation", operation),
            status,
            text_field("error_code", failure.code),
            text_field("error", &failure.message),
        ],
    );
}
fn surface() -> abi::SurfaceInfo {
    abi::SurfaceInfo {
        kind: abi::SURFACE_CPU,
        pixel_format: abi::PIXEL_BGRA8,
        color: abi::COLOR_SRGB,
        alpha: abi::ALPHA_OPAQUE,
        rows: abi::ROWS_TOP_DOWN,
        logical: abi::Extent {
            width: 1024,
            height: 640,
        },
        raster: abi::Extent {
            width: 1024,
            height: 640,
        },
        stride: 4096,
    }
}
struct Formats([abi::Bytes; 1]);
unsafe impl Sync for Formats {}
static FORMATS: Formats = Formats([abi::Bytes::from_static(b"musica.paz")]);
const FRAME_POOL_SLOTS: usize = 3;
const FRAME_BYTES: usize = 1024 * 640 * 4;
struct FramePool {
    slots: [Arc<Vec<u8>>; FRAME_POOL_SLOTS],
}
impl FramePool {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| Arc::new(vec![0; FRAME_BYTES])),
        }
    }
    fn writable(&mut self) -> Option<(usize, &mut [u8])> {
        self.slots.iter_mut().enumerate().find_map(|(index, slot)| {
            Arc::get_mut(slot).map(|pixels| (index, pixels.as_mut_slice()))
        })
    }
}
struct View {
    state: u32,
    error: abi::Error,
    generation: u64,
    timestamp: u64,
    pixels: Option<Arc<Vec<u8>>>,
}
struct Shared {
    host: Host,
    view: Mutex<View>,
    tx: mpsc::SyncSender<Command>,
}
enum Command {
    Inputs(Vec<NativeInput>),
    Pause(bool, mpsc::SyncSender<Result<(), Failure>>),
}
enum NativeInput {
    Event(InputEvent),
    PointerButton(InputEvent, f32, f32),
    Window(bool, bool),
}
struct ClickTracker {
    cursor: (f32, f32),
    previous: Option<(Instant, f32, f32)>,
}
impl ClickTracker {
    fn apply(&mut self, event: &mut InputEvent) {
        match event {
            InputEvent::PointerMoved { x, y } => self.cursor = (*x, *y),
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ButtonState::Pressed,
                click_count,
            } => {
                #[cfg(windows)]
                let (interval, dx, dy) = eden_engine::platform::windows::double_click_limits();
                #[cfg(not(windows))]
                let (interval, dx, dy) = (Duration::from_millis(500), 4.0, 4.0);
                let now = Instant::now();
                let double = self.previous.is_some_and(|(time, x, y)| {
                    now.duration_since(time) <= interval
                        && (x - self.cursor.0).abs() <= dx
                        && (y - self.cursor.1).abs() <= dy
                });
                *click_count = if double { 2 } else { 1 };
                self.previous = if double {
                    None
                } else {
                    Some((now, self.cursor.0, self.cursor.1))
                };
            }
            _ => {}
        }
    }
}
struct Handle {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
fn handle<'a>(session: abi::Session) -> Result<&'a Handle, Failure> {
    if session.is_null() || !(session as *const Handle).is_aligned() {
        return Err(Failure::invalid("null or misaligned session"));
    }
    Ok(unsafe { &*(session as *const Handle) })
}
fn fail(shared: &Shared, failure: &Failure) {
    let mut view = shared.view.lock().unwrap_or_else(|p| p.into_inner());
    view.state = abi::SESSION_FAILED;
    view.error = failure.error();
}
fn evidence(engine: &Engine, consumed: u64) {
    let phase = match &engine.phase {
        eden_engine::Phase::Title { .. } => 1,
        eden_engine::Phase::Game => 2,
        eden_engine::Phase::SaveLoad(_) => 3,
        eden_engine::Phase::Memories { .. } => 4,
        eden_engine::Phase::CgGallery(_) => 5,
        eden_engine::Phase::MusicMenu(_) => 6,
        eden_engine::Phase::Config(..) => 7,
    };
    let (pc, wait, frames, voice) = engine.vm.as_ref().map_or((0, 0, 0, false), |vm| {
        let (wait, frames, voice) = match vm.state() {
            RunState::Running => (1, 0, false),
            RunState::WaitFrames(frames) => (2, u64::from(*frames), false),
            RunState::AwaitInput => (3, 0, false),
            RunState::AwaitAutoMessage { wait_voice } => (4, 0, *wait_voice),
            RunState::WaitShake => (5, 0, false),
            RunState::WaitScroll => (6, 0, false),
            RunState::Ended => (7, 0, false),
        };
        (u64::from(vm.pc()), wait, frames, voice)
    });
    emit_event(
        abi::LOG_INFO,
        "reminori.native_state",
        &[
            numeric_field("phase", phase),
            numeric_field("input_count", consumed),
            numeric_field("pc", pc),
            numeric_field("wait_kind", wait),
            numeric_field("wait_frames", frames),
            numeric_field("wait_voice", u64::from(voice)),
        ],
    );
}
fn worker(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Command>,
    settings: config::Configuration,
    opened: mpsc::SyncSender<Result<(), Failure>>,
    window: abi::WindowState,
) {
    let mut initialized = false;
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), Failure> {
        if shared.host.cancelled() {
            return Err(Failure::status(abi::CANCELLED));
        }
        let mut engine = Engine::new_offscreen(
            settings.game,
            settings.profile,
            settings.saves,
            settings.null_audio,
            &|| shared.host.cancelled(),
        )
        .map_err(|error| {
            if shared.host.cancelled() {
                Failure::status(abi::CANCELLED)
            } else {
                Failure::runtime(format!("{error:#}"))
            }
        })?;
        if shared.host.cancelled() {
            engine.shutdown();
            return Err(Failure::status(abi::CANCELLED));
        }
        if window.focused == 0 {
            engine.notify_focus_lost();
        }
        engine
            .set_paused(window.suspended != 0)
            .map_err(Failure::runtime)?;
        let mut pool = FramePool::new();
        let (first_slot, pixels) = pool
            .writable()
            .expect("new frame pool is exclusively owned");
        engine.render_cpu_into(pixels).map_err(Failure::runtime)?;
        {
            let mut view = shared.view.lock().unwrap_or_else(|p| p.into_inner());
            view.pixels = Some(Arc::clone(&pool.slots[first_slot]));
            view.generation = 1;
            view.state = abi::SESSION_RUNNING;
        }
        evidence(&engine, 0);
        initialized = true;
        let _ = opened.send(Ok(()));
        let start = Instant::now();
        let mut previous = Instant::now();
        let mut deadline = previous;
        let mut paused = false;
        let mut suspended = window.suspended != 0;
        let mut consumed = 0;
        let mut clicks = ClickTracker {
            cursor: (0.0, 0.0),
            previous: None,
        };
        let mut next_evidence = Instant::now() + Duration::from_secs(5);
        loop {
            if shared.host.cancelled() || engine.quit {
                break;
            }
            let timeout = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(timeout) {
                Ok(Command::Inputs(events)) => {
                    for event in events {
                        match event {
                            NativeInput::Event(mut event) => {
                                clicks.apply(&mut event);
                                engine.input(event);
                                consumed += 1;
                            }
                            NativeInput::PointerButton(mut event, x, y) => {
                                if clicks.cursor != (x, y) {
                                    let mut moved = InputEvent::PointerMoved { x, y };
                                    clicks.apply(&mut moved);
                                    engine.input(moved);
                                }
                                clicks.apply(&mut event);
                                engine.input(event);
                                consumed += 1;
                            }
                            NativeInput::Window(focused, is_suspended) => {
                                if focused {
                                    engine.notify_focus_gained();
                                } else {
                                    engine.notify_focus_lost();
                                }
                                suspended = is_suspended;
                                engine
                                    .set_paused(paused || suspended)
                                    .map_err(Failure::runtime)?;
                            }
                        }
                    }
                    evidence(&engine, consumed);
                }
                Ok(Command::Pause(value, response)) => {
                    let result = engine
                        .set_paused(value || suspended)
                        .map_err(Failure::runtime);
                    if result.is_ok() {
                        paused = value;
                        shared.view.lock().unwrap_or_else(|p| p.into_inner()).state = if paused {
                            abi::SESSION_PAUSED
                        } else {
                            abi::SESSION_RUNNING
                        };
                    }
                    let _ = response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let platform_result = platform::pump(&mut engine, &shared.host);
            if shared.host.cancelled() {
                break;
            }
            platform_result?;
            let now = Instant::now();
            if now >= deadline {
                if !paused && !suspended {
                    engine.advance_time(now.saturating_duration_since(previous));
                    let platform_result = platform::pump(&mut engine, &shared.host);
                    if shared.host.cancelled() {
                        break;
                    }
                    platform_result?;
                    if let Some((slot, pixels)) = pool.writable() {
                        engine.render_cpu_into(pixels).map_err(Failure::runtime)?;
                        let mut view = shared.view.lock().unwrap_or_else(|p| p.into_inner());
                        view.pixels = Some(Arc::clone(&pool.slots[slot]));
                        view.generation = view.generation.saturating_add(1);
                        view.timestamp = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                    } else {
                        // Leased slots are immutable. Preserve the native Engine
                        // loop and final GPU draw while dropping this display frame.
                        engine.render_without_readback().map_err(Failure::runtime)?;
                    }
                }
                previous = now;
                deadline = now + Duration::from_nanos(16_666_667);
            }
            if now >= next_evidence {
                evidence(&engine, consumed);
                next_evidence = now + Duration::from_secs(5);
            }
        }
        engine.shutdown();
        shared.view.lock().unwrap_or_else(|p| p.into_inner()).state = abi::SESSION_FINISHED;
        Ok(())
    }));
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some(Failure::panic()),
    };
    if let Some(failure) = failure {
        fail(&shared, &failure);
        if !initialized {
            let _ = opened.send(Err(failure));
        }
    }
}

unsafe extern "C" fn initialize(
    diagnostics: *const abi::Diagnostics,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        let diagnostics = unsafe { diagnostics.as_ref() }
            .ok_or_else(|| Failure::invalid("missing diagnostics"))?;
        if !diagnostics.header.accepts::<abi::Diagnostics>() {
            return Err(Failure::status(abi::UNSUPPORTED));
        }
        if DIAGNOSTICS.get().is_none() {
            log::set_logger(&LOGGER).map_err(Failure::runtime)?;
            let _ = DIAGNOSTICS.set(DiagnosticSink(*diagnostics));
            log::set_max_level(log::LevelFilter::Info);
        }
        Ok(())
    })
}
unsafe extern "C" fn describe(output: *mut abi::Descriptor, error: *mut abi::Error) -> abi::Status {
    boundary(error, || {
        if output.is_null() {
            return Err(Failure::invalid("missing descriptor output"));
        }
        let fields = config::fields();
        unsafe {
            output.write(abi::Descriptor {
                header: abi::Header::of::<abi::Descriptor>(),
                family_id: bytes("musica"),
                plugin_id: bytes("ReMinori"),
                version: bytes(env!("CARGO_PKG_VERSION")),
                capabilities: abi::CAP_CPU_SURFACE,
                formats: FORMATS.0.as_ptr(),
                format_count: FORMATS.0.len(),
                configuration: fields.as_ptr(),
                configuration_count: fields.len(),
                display_name: bytes("ReMinori"),
                family_name: bytes("Musica"),
                render: abi::RenderExtentSupport::default(),
            });
        }
        Ok(())
    })
}
unsafe extern "C" fn probe(
    path: abi::Bytes,
    output: *mut abi::ProbeResult,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        if output.is_null() {
            return Err(Failure::invalid("missing probe output"));
        }
        unsafe {
            output.write(abi::ProbeResult {
                game_id: abi::Bytes::EMPTY,
                format: abi::Bytes::EMPTY,
                confidence_permyriad: 0,
                reserved: 0,
            });
        }
        let path = Path::new(unsafe { path.text(32768) }.map_err(Failure::status)?);
        let root = if path.is_dir() {
            path
        } else {
            path.parent()
                .ok_or_else(|| Failure::status(abi::NO_MATCH))?
        };
        if !root.join("scr.paz").is_file()
            || !root.join("sys.paz").is_file()
            || !["eden.exe", "eden_en.exe"]
                .iter()
                .any(|file| root.join(file).is_file())
        {
            return Err(Failure::status(abi::NO_MATCH));
        }
        unsafe {
            output.write(abi::ProbeResult {
                game_id: bytes("eden"),
                format: bytes("musica.paz"),
                confidence_permyriad: 9000,
                reserved: 0,
            });
        }
        Ok(())
    })
}
unsafe extern "C" fn open(
    request: *const abi::OpenRequest,
    output: *mut abi::OpenResult,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        if output.is_null() {
            return Err(Failure::invalid("missing open output"));
        }
        unsafe {
            output.write(abi::OpenResult {
                session: std::ptr::null_mut(),
                context_epoch: 0,
                output_epoch: 0,
                surface: surface(),
            });
        }
        let request =
            unsafe { request.as_ref() }.ok_or_else(|| Failure::invalid("missing open request"))?;
        if !request.header.accepts::<abi::OpenRequest>()
            || !request.host.header.accepts::<abi::HostServices>()
        {
            return Err(Failure::status(abi::UNSUPPORTED));
        }
        if request.session_epoch == 0
            || request.host.cancellation.is_cancelled.is_none()
            || !request.module_owner.is_null()
            || request.render_extent != abi::Extent::default()
            || request.window.focused > 1
            || request.window.suspended > 1
            || !request.window.scale_factor.is_finite()
            || request.window.scale_factor <= 0.0
        {
            return Err(Failure::invalid(
                "invalid open epoch, services, module owner, render extent or window state",
            ));
        }
        let settings = unsafe { config::parse(request) }?;
        let host = Host {
            services: request.host,
            epoch: request.session_epoch,
            local_cancel: Arc::new(AtomicBool::new(false)),
        };
        if host.cancelled() {
            return Err(Failure::status(abi::CANCELLED));
        }
        let (tx, rx) = mpsc::sync_channel(128);
        let shared = Arc::new(Shared {
            host,
            tx,
            view: Mutex::new(View {
                state: abi::SESSION_WAITING,
                error: abi::Error::default(),
                generation: 0,
                timestamp: 0,
                pixels: None,
            }),
        });
        let (opened_tx, opened_rx) = mpsc::sync_channel(1);
        let owned_shared = Arc::clone(&shared);
        let window = request.window;
        let worker = thread::Builder::new()
            .name("shinki-ReMinori".into())
            .spawn(move || worker(owned_shared, rx, settings, opened_tx, window))
            .map_err(Failure::runtime)?;
        let initialization = opened_rx.recv().map_err(Failure::runtime).and_then(|r| r);
        if initialization.is_err() || shared.host.cancelled() {
            shared.host.cancel();
            let _ = worker.join();
            return initialization.and(Err(Failure::status(abi::CANCELLED)));
        }
        let handle = Box::new(Handle {
            shared,
            worker: Some(worker),
        });
        unsafe {
            output.write(abi::OpenResult {
                session: Box::into_raw(handle).cast(),
                context_epoch: 1,
                output_epoch: 1,
                surface: surface(),
            });
        }
        Ok(())
    })
}

unsafe fn convert(event: &abi::InputEvent, epoch: u64) -> Result<NativeInput, Failure> {
    if event.pressed > 1
        || event.repeat > 1
        || !event.x.is_finite()
        || !event.y.is_finite()
        || event.x.abs() > f32::MAX as f64
        || event.y.abs() > f32::MAX as f64
    {
        return Err(Failure::invalid("invalid input state or coordinates"));
    }
    let state = if event.pressed == 1 {
        ButtonState::Pressed
    } else {
        ButtonState::Released
    };
    if matches!(
        event.kind,
        abi::INPUT_POINTER_MOVE | abi::INPUT_POINTER_BUTTON
    ) && (event.displayed.session_epoch != epoch
        || event.displayed.output_epoch != 1
        || event.displayed.generation == 0)
    {
        return Err(Failure::status(abi::STALE_CONTEXT));
    }
    let native = match event.kind {
        abi::INPUT_KEY => {
            let physical = unsafe { event.physical_key.text(128) }.map_err(Failure::status)?;
            let logical = unsafe { event.logical_key.text(128) }.map_err(Failure::status)?;
            let key = match physical {
                "Enter" | "NumpadEnter" => InputKey::Enter,
                "Space" => InputKey::Space,
                "Escape" => InputKey::Escape,
                "ArrowUp" => InputKey::ArrowUp,
                "ArrowDown" => InputKey::ArrowDown,
                "ArrowLeft" => InputKey::ArrowLeft,
                "ArrowRight" => InputKey::ArrowRight,
                "PageUp" => InputKey::PageUp,
                "PageDown" => InputKey::PageDown,
                "Home" => InputKey::Home,
                "End" => InputKey::End,
                "ControlLeft" | "ControlRight" => InputKey::Control,
                "F5" => InputKey::F5,
                "F9" => InputKey::F9,
                _ => match logical {
                    "Enter" => InputKey::Enter,
                    "Escape" => InputKey::Escape,
                    " " => InputKey::Space,
                    "Control" => InputKey::Control,
                    other => InputKey::Character(other.into()),
                },
            };
            InputEvent::Key {
                key,
                state,
                repeat: event.repeat == 1,
            }
        }
        abi::INPUT_POINTER_MOVE => InputEvent::PointerMoved {
            x: event.x as f32,
            y: event.y as f32,
        },
        abi::INPUT_POINTER_BUTTON => {
            let button = match event.button {
                abi::BUTTON_PRIMARY => PointerButton::Left,
                abi::BUTTON_SECONDARY => PointerButton::Right,
                abi::BUTTON_MIDDLE => PointerButton::Middle,
                _ => return Err(Failure::invalid("unsupported pointer button")),
            };
            return Ok(NativeInput::PointerButton(
                InputEvent::PointerButton {
                    button,
                    state,
                    click_count: 1,
                },
                event.x as f32,
                event.y as f32,
            ));
        }
        abi::INPUT_WHEEL => InputEvent::WheelSteps {
            x: event.x as f32,
            y: event.y as f32,
        },
        abi::INPUT_WINDOW => {
            if event.window.focused > 1
                || event.window.suspended > 1
                || !event.window.scale_factor.is_finite()
                || event.window.scale_factor <= 0.0
            {
                return Err(Failure::invalid("invalid window input"));
            }
            return Ok(NativeInput::Window(
                event.window.focused != 0,
                event.window.suspended != 0,
            ));
        }
        _ => return Err(Failure::invalid("unknown input event")),
    };
    Ok(NativeInput::Event(native))
}
unsafe extern "C" fn input(
    session: abi::Session,
    events: *const abi::InputEvent,
    count: usize,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        let handle = handle(session)?;
        if handle.shared.host.cancelled() {
            return Err(Failure::status(abi::CANCELLED));
        }
        let events =
            unsafe { abi::borrowed_array(events, count, 4096) }.map_err(Failure::status)?;
        let events = events
            .iter()
            .map(|event| unsafe { convert(event, handle.shared.host.epoch) })
            .collect::<Result<Vec<_>, _>>()?;
        handle
            .shared
            .tx
            .try_send(Command::Inputs(events))
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => Failure::status(abi::CAPACITY),
                mpsc::TrySendError::Disconnected(_) => Failure::status(abi::INVALID_STATE),
            })?;
        Ok(())
    })
}
unsafe extern "C" fn control(
    session: abi::Session,
    command: u32,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        let handle = handle(session)?;
        if handle.shared.host.cancelled() {
            return Err(Failure::status(abi::CANCELLED));
        }
        let paused = match command {
            abi::CONTROL_PAUSE => true,
            abi::CONTROL_RESUME => false,
            _ => return Err(Failure::status(abi::UNSUPPORTED)),
        };
        let (tx, rx) = mpsc::sync_channel(1);
        handle
            .shared
            .tx
            .try_send(Command::Pause(paused, tx))
            .map_err(|_| Failure::status(abi::INVALID_STATE))?;
        rx.recv().map_err(Failure::runtime)?
    })
}
unsafe extern "C" fn state(
    session: abi::Session,
    output: *mut abi::SessionState,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        let handle = handle(session)?;
        if output.is_null() {
            return Err(Failure::invalid("missing state output"));
        }
        let view = handle.shared.view.lock().unwrap_or_else(|p| p.into_inner());
        unsafe {
            output.write(abi::SessionState {
                state: view.state,
                flags: 0,
                context_epoch: 1,
                output_epoch: 1,
                error: view.error,
            });
        }
        Ok(())
    })
}
unsafe extern "C" fn release(owner: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !owner.is_null() {
            drop(unsafe { Arc::from_raw(owner.cast::<Vec<u8>>()) });
        }
    }));
}
unsafe extern "C" fn frame(
    session: abi::Session,
    previous: abi::FrameIdentity,
    output: *mut abi::FrameSurface,
    error: *mut abi::Error,
) -> abi::Status {
    boundary(error, || {
        let handle = handle(session)?;
        if output.is_null() {
            return Err(Failure::invalid("missing frame output"));
        }
        let view = handle.shared.view.lock().unwrap_or_else(|p| p.into_inner());
        let identity = abi::FrameIdentity {
            session_epoch: handle.shared.host.epoch,
            output_epoch: 1,
            generation: view.generation,
        };
        let mut frame = abi::FrameSurface {
            state: abi::FRAME_PENDING,
            reserved: 0,
            identity,
            timestamp_ns: view.timestamp,
            info: surface(),
            cpu: abi::CpuLease {
                pixels: abi::Bytes::EMPTY,
                owner: std::ptr::null_mut(),
                release: None,
            },
        };
        if previous == identity && view.generation != 0 {
            frame.state = abi::FRAME_UNCHANGED;
        } else if let Some(pixels) = &view.pixels {
            frame.state = abi::FRAME_READY;
            frame.cpu.pixels = abi::Bytes::new(pixels.as_slice());
            frame.cpu.owner = Arc::into_raw(Arc::clone(pixels)).cast_mut().cast();
            frame.cpu.release = Some(release);
        }
        unsafe {
            output.write(frame);
        }
        Ok(())
    })
}
unsafe extern "C" fn cancel(session: abi::Session) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(handle) = handle(session) {
            handle.shared.host.cancel();
        }
    }));
}
unsafe extern "C" fn close(session: abi::Session, error: *mut abi::Error) -> abi::Status {
    boundary(error, || {
        if session.is_null() || !(session as *const Handle).is_aligned() {
            return Err(Failure::invalid("null or misaligned session"));
        }
        let mut handle = unsafe { Box::from_raw(session.cast::<Handle>()) };
        handle.shared.host.cancel();
        if let Some(worker) = handle.worker.take() {
            worker.join().map_err(|_| Failure::panic())?;
        }
        Ok(())
    })
}
unsafe extern "C" fn extension(
    _id: abi::Bytes,
    _version: u32,
    output: *mut *const c_void,
) -> abi::Status {
    if output.is_null() {
        return abi::INVALID_ARGUMENT;
    }
    unsafe {
        output.write(std::ptr::null());
    }
    abi::UNSUPPORTED
}
static API: abi::CoreApi = abi::CoreApi {
    header: abi::Header::of::<abi::CoreApi>(),
    initialize: Some(initialize),
    describe: Some(describe),
    probe: Some(probe),
    open: Some(open),
    input: Some(input),
    control: Some(control),
    state: Some(state),
    frame: Some(frame),
    cancel: Some(cancel),
    close: Some(close),
    extension: Some(extension),
};
/// Query the immutable native C ABI function table.
///
/// # Safety
/// A non-null `output` must be aligned and writable for one table pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shinki_emu_query(
    version: u32,
    output: *mut *const abi::CoreApi,
) -> abi::Status {
    if output.is_null() {
        return abi::INVALID_ARGUMENT;
    }
    unsafe {
        output.write(std::ptr::null());
    }
    if version != abi::ABI_VERSION {
        return abi::UNSUPPORTED;
    }
    unsafe {
        output.write(&API);
    }
    abi::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_and_descriptor_are_v3_valid() {
        unsafe { abi::validate_config(config::fields(), &[]) }.unwrap();
        let mut descriptor = std::mem::MaybeUninit::uninit();
        assert_eq!(
            unsafe { describe(descriptor.as_mut_ptr(), std::ptr::null_mut()) },
            abi::OK
        );
        unsafe { descriptor.assume_init().validate() }.unwrap();
        surface().validate().unwrap();
        API.validate().unwrap();
    }
    #[test]
    fn query_clears_incompatible_output() {
        let mut output = &API as *const _;
        assert_eq!(unsafe { shinki_emu_query(2, &mut output) }, abi::UNSUPPORTED);
        assert!(output.is_null());
    }
    #[test]
    fn cpu_lease_owns_only_immutable_pixels() {
        let pixels = Arc::new(vec![7u8; 16]);
        let owner = Arc::clone(&pixels);
        drop(pixels);
        let ptr = Arc::into_raw(owner);
        assert_eq!(unsafe { &*ptr }, &[7; 16]);
        unsafe {
            release(ptr.cast_mut().cast());
        }
    }
    #[test]
    fn fixed_frame_pool_never_overwrites_leased_slots() {
        let mut pool = FramePool::new();
        let [first, second, third] = pool.slots.clone();
        assert!(pool.writable().is_none());
        drop(first);
        {
            let (slot, pixels) = pool.writable().unwrap();
            assert_eq!(slot, 0);
            pixels[0] = 99;
        }
        assert_eq!(second[0], 0);
        assert_eq!(third[0], 0);
        drop(pool);
        assert_eq!(second.len(), FRAME_BYTES);
        assert_eq!(third[0], 0);
    }
}
