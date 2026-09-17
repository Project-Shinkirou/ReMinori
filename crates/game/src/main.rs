#![cfg_attr(
    all(windows, not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]
use eden_engine::{Engine, InputEvent};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
mod input_adapter;
mod platform_dispatcher;
use input_adapter::InputAdapter;
use platform_dispatcher::dispatch as dispatch_platform;
struct Cli {
    game_root: PathBuf,
    override_root: Option<PathBuf>,
    load: Option<String>,
}
fn parse_cli() -> Result<Cli, String> {
    let mut game_root = None;
    let mut override_root = None;
    let mut load = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game-root" => {
                game_root = Some(
                    PathBuf::from(args.next().ok_or("--game-root 缺少目录参数")?),
                );
            }
            "--override" => {
                override_root = Some(
                    PathBuf::from(args.next().ok_or("--override 缺少目录参数")?),
                );
            }
            _ if !arg.starts_with('-') => {
                if load.is_none() {
                    load = Some(arg);
                }
            }
            other => return Err(format!("未知参数 {other}")),
        }
    }
    let game_root = match game_root {
        Some(p) => p,
        None => {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
                .ok_or(
                    "无法定位 exe 所在目录，请用 --game-root 指定数据目录",
                )?
        }
    };
    if !game_root.join("scr.paz").is_file() {
        return Err(
            format!(
                "{} 下没有 scr.paz：--game-root 应指向 eden* 数据目录",
                game_root.display()
            ),
        );
    }
    Ok(Cli {
        game_root,
        override_root,
        load,
    })
}
fn win_size() -> winit::dpi::Size {
    if let Some(v) = std::env::var_os("EDEN_SIZE") {
        if let Some((w, h)) = v.to_string_lossy().split_once(['x', 'X', '*']) {
            if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                if w > 0 && h > 0 {
                    return winit::dpi::PhysicalSize::new(w, h).into();
                }
            }
        }
    }
    winit::dpi::PhysicalSize::new(1024, 640).into()
}
struct App {
    engine: Option<Engine>,
    window: Option<Arc<Window>>,
    input: InputAdapter,
    last: Instant,
    cli: Cli,
}
impl App {
    fn new(cli: Cli) -> Self {
        App {
            engine: None,
            window: None,
            input: InputAdapter::new(),
            last: Instant::now(),
            cli,
        }
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("eden")
                        .with_inner_size(win_size()),
                )
                .expect("窗口创建失败"),
        );
        let size = window.inner_size();
        let mut engine = Engine::new_with_override(
                window.clone(),
                self.cli.game_root.clone(),
                self.cli.override_root.clone(),
                size.width,
                size.height,
            )
            .expect("引擎初始化失败");
        if let Ok(v) = std::env::var("EDEN_SHOT") {
            for part in v.split(';') {
                if let Some((ms, p)) = part.split_once(':') {
                    if let (Ok(ms), p) = (ms.trim().parse::<u64>(), p.trim()) {
                        engine
                            .shots
                            .push((
                                std::time::Duration::from_millis(ms),
                                PathBuf::from(p),
                            ));
                    }
                }
            }
        }
        if let Ok(v) = std::env::var("EDEN_AUTO") {
            if let Ok(ms) = v.parse::<u64>() {
                engine.auto_advance_every = Some(std::time::Duration::from_millis(ms));
            }
        }
        if let Ok(v) = std::env::var("EDEN_EXIT") {
            if let Ok(ms) = v.parse::<u64>() {
                engine.exit_after = Some(std::time::Duration::from_millis(ms));
            }
        }
        if let Ok(v) = std::env::var("EDEN_CLICK") {
            for part in v.split(';') {
                if let Ok(ms) = part.trim().parse::<u64>() {
                    engine.clicks_at.push(std::time::Duration::from_millis(ms));
                }
            }
        }
        if let Ok(v) = std::env::var("EDEN_HOVER") {
            if let Some((x, y)) = v.split_once(',') {
                if let (Ok(x), Ok(y)) = (
                    x.trim().parse::<f32>(),
                    y.trim().parse::<f32>(),
                ) {
                    engine.input(InputEvent::PointerMoved { x, y });
                }
            }
        }
        if let Ok(v) = std::env::var("EDEN_HOVER_AT") {
            for part in v.split(';') {
                if let Some((ms, xy)) = part.split_once(':') {
                    if let Some((x, y)) = xy.split_once(',') {
                        if let (Ok(ms), Ok(x), Ok(y)) = (
                            ms.trim().parse::<u64>(),
                            x.trim().parse::<f32>(),
                            y.trim().parse::<f32>(),
                        ) {
                            engine
                                .hovers_at
                                .push((std::time::Duration::from_millis(ms), x, y));
                        }
                    }
                }
            }
        }
        if let Ok(v) = std::env::var("EDEN_SAVE") {
            if let Some((ms, p)) = v.split_once(':') {
                if let Ok(ms) = ms.trim().parse::<u64>() {
                    engine.save_at = Some((
                        std::time::Duration::from_millis(ms),
                        std::path::PathBuf::from(p.trim()),
                    ));
                }
            }
        }
        let load_path = self
            .cli
            .load
            .clone()
            .or_else(|| std::env::var("EDEN_LOAD").ok());
        if let Some(path) = load_path {
            match engine.load(std::path::Path::new(&path)) {
                Ok(()) => log::info!("启动读档 ← {path}"),
                Err(e) => log::error!("读档失败 {path}: {e:?}"),
            }
        }
        self.window = Some(window);
        self.engine = Some(engine);
        self.last = Instant::now();
    }
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(engine) = &mut self.engine else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => engine.request_exit(),
            WindowEvent::Focused(false) => engine.notify_focus_lost(),
            WindowEvent::Focused(true) => engine.notify_focus_gained(),
            WindowEvent::Resized(size) => engine.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                let dt = self.last.elapsed();
                self.last = Instant::now();
                engine.advance_time(dt);
                engine.render();
                if engine.quit {
                    event_loop.exit();
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(input) = self.input.keyboard(&event) {
                    engine.input(input);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                engine.input(self.input.mouse_button(button, state));
            }
            WindowEvent::CursorMoved { position, .. } => {
                engine.input(self.input.pointer_moved(window, position));
            }
            WindowEvent::CursorLeft { .. } => engine.input(self.input.pointer_left()),
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(input) = self.input.wheel(delta) {
                    engine.input(input);
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _ = event_loop;
        if let Some(engine) = &mut self.engine {
            let dt = self.last.elapsed();
            self.last = Instant::now();
            engine.advance_time(dt);
            engine.render();
            if let Some(w) = &self.window {
                dispatch_platform(engine, w);
            }
            if engine.quit {
                event_loop.exit();
            }
        }
    }
}
fn main() -> anyhow::Result<()> {
    init_logging();
    let cli = match parse_cli() {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: eden [--game-root DIR] [--override DIR] [存档.sav]");
            log::error!("{message}");
            std::process::exit(2);
        }
    };
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(cli);
    event_loop.run_app(&mut app)?;
    Ok(())
}
fn init_logging() {
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu=error,naga=error"),
    );
    builder.filter_module("symphonia_format_ogg", log::LevelFilter::Warn);
    if let Some(file) = console_less_log_file() {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.init();
}
fn console_less_log_file() -> Option<std::fs::File> {
    if !cfg!(all(windows, not(debug_assertions), not(feature = "console"))) {
        return None;
    }
    let dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("eden")
        .join("logs");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("无法创建日志目录 {}: {error}", dir.display());
        return None;
    }
    let path = dir.join("eden.log");
    if path.is_file() {
        let _ = std::fs::rename(&path, dir.join("eden.prev.log"));
    }
    match std::fs::File::create(&path) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("无法创建日志文件 {}: {error}", path.display());
            None
        }
    }
}
