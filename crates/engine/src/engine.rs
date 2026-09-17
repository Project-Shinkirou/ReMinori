#[cfg(not(target_arch = "wasm32"))]
use crate::audio::cpal_backend::CpalAudio;
#[cfg(not(target_arch = "wasm32"))]
use crate::audio::NullAudio;
use crate::audio::{parse_vol_pan, AudioBackend, AuxChannel};
use crate::backlog::{BacklogView, GAUGE_RECT};
use crate::cgmode::{CgGallery, CgSeen, CgViewer};
use crate::font::GlyphAtlas;
use crate::host::EngineHost;
use crate::input::{ButtonState, InputEvent, InputKey, PointerButton};
use crate::music::MusicMenu;
use crate::platform::api::{
    ContextMenuCommand, ContextMenuState, PlatformRequest, PlatformResponse, RequestId,
    ScreenMode, WindowGeometry,
};
use crate::render::gpu::{
    EffectSpriteDraw, FrameDraw, MaskTransitionDraw, Renderer, SpriteDraw,
};
use crate::render::text::{
    layout_ex, speaker_display_for_panel, GlyphQuad, TextBox, MSG_BOX_BAR, MSG_BOX_FULL,
    MSG_BOX_FULL2, MSG_BOX_PANEL1, NAME_BOX_BAR, NAME_BOX_FULL, NAME_BOX_PANEL1,
};
use crate::render::Viewport;
use crate::resource::ResourceStore;
use crate::saveload::{SaveLoadMenu, SLOT_RECTS};
use crate::scene::Scene;
use crate::settings::Settings;
use crate::storage::StorageBackend;
use crate::vfs::Vfs;
#[cfg(not(target_arch = "wasm32"))]
use crate::video::NativeVideoBackend;
use crate::video::{MovieStatus, MovieTarget, VideoBackend};
use eden_vm::{RunState, Scheduler, Step, Vm};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;
static SAVE_DIR: OnceLock<PathBuf> = OnceLock::new();
const USER_DATA_DIR: &str = "userdata";
fn resolve_save_dir() -> PathBuf {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        let dir = exe_dir.join(USER_DATA_DIR);
        if std::fs::create_dir_all(&dir).is_ok() && probe_writable(&dir) {
            log::info!("用户数据目录（exe 同目录）: {}", dir.display());
            return dir;
        }
        log::warn!(
            "exe 同目录不可写 {}，用户数据回退文档目录", dir.display()
        );
    }
    let dir = dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("eden")
        .join(USER_DATA_DIR);
    log::info!("用户数据目录（文档目录）: {}", dir.display());
    dir
}
fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(".eden-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
const TITLE_MENU: &[(&str, bool)] = &[
    ("NewGame", true),
    ("LoadGame", true),
    ("System", true),
    ("Memories", true),
    ("Mosaic", false),
    ("Exit", true),
];
const TITLE_HIT: [[f32; 4]; 6] = [
    [850.0, 100.0, 1024.0, 128.0],
    [850.0, 128.0, 1024.0, 157.0],
    [850.0, 157.0, 1024.0, 186.0],
    [850.0, 186.0, 1024.0, 214.0],
    [850.0, 214.0, 1024.0, 243.0],
    [850.0, 243.0, 1024.0, 271.0],
];
fn title_bg_index(has_progress: bool, has_op_seen: bool) -> u8 {
    if has_progress { 2 } else if has_op_seen { 1 } else { 0 }
}
const MEMORIES_MENU: &[(&str, bool)] = &[
    ("Snaps", true),
    ("Movie", true),
    ("Music", true),
    ("Scene", false),
    ("Return", true),
];
const MEMORIES_HIT: [[f32; 4]; 5] = [
    [640.0, 84.0, 1024.0, 162.0],
    [640.0, 162.0, 1024.0, 242.0],
    [640.0, 242.0, 1024.0, 322.0],
    [640.0, 322.0, 1024.0, 400.0],
    [804.0, 562.0, 988.0, 628.0],
];
#[derive(Debug, Clone, Copy)]
enum Action {
    Advance,
    Click { click_count: u8 },
    MouseRelease,
    RightClick,
    MiddleRelease,
    MouseMove { x: f32, y: f32 },
    MouseLeft,
    MouseWheel { dx: f32, dy: f32 },
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    HomeKey,
    EndKey,
    Cancel,
    ToggleMessagePanel,
    CtrlHeld(bool),
}
pub enum Phase {
    Title { selection: usize, hover: Option<usize> },
    Game,
    SaveLoad(Box<SaveLoadMenu>),
    Memories { hover: Option<usize> },
    CgGallery(Box<CgGallery>),
    MusicMenu(Box<MusicMenu>),
    Config(Box<crate::config::ConfigState>, Box<Phase>),
}
struct ShakeSnapshot {
    draw: FrameDraw,
    mask: Option<MaskTransitionDraw>,
}
pub enum FontSource {
    Bytes { bytes: Vec<u8>, name: String },
    System { preferred: String },
}
impl FontSource {
    pub fn bytes(bytes: Vec<u8>, name: impl Into<String>) -> Self {
        Self::Bytes {
            bytes,
            name: name.into(),
        }
    }
    fn load(self) -> anyhow::Result<GlyphAtlas> {
        match self {
            Self::Bytes { bytes, name } => GlyphAtlas::from_bytes_named(bytes, name),
            Self::System { preferred } => GlyphAtlas::from_system_named(&preferred),
        }
    }
}
pub struct EngineServices {
    pub resources: Rc<dyn ResourceStore>,
    pub native_vfs: Option<Rc<Vfs>>,
    pub audio: Rc<RefCell<dyn AudioBackend>>,
    pub storage: Rc<dyn StorageBackend>,
    pub video: Rc<RefCell<dyn VideoBackend>>,
    pub font: FontSource,
    pub settings: Rc<RefCell<Settings>>,
    pub cg_seen: Rc<RefCell<CgSeen>>,
    pub read_history: Rc<RefCell<crate::read_history::ReadHistory>>,
    pub progress: std::collections::BTreeMap<String, String>,
}
pub struct Engine {
    pub renderer: Renderer,
    pub resources: Rc<dyn ResourceStore>,
    native_vfs: Option<Rc<Vfs>>,
    pub scene: Rc<RefCell<Scene>>,
    pub vm: Option<Vm<EngineHost>>,
    pub scheduler: Scheduler,
    pub atlas: GlyphAtlas,
    pub phase: Phase,
    pub quit: bool,
    fast_forward: bool,
    focused: bool,
    cursor: Option<(f32, f32)>,
    title_fade: u8,
    frame_no: u64,
    next_auto: std::time::Duration,
    pub shots: Vec<(std::time::Duration, PathBuf)>,
    pub auto_advance_every: Option<std::time::Duration>,
    pub exit_after: Option<std::time::Duration>,
    pub clicks_at: Vec<std::time::Duration>,
    pub hovers_at: Vec<(std::time::Duration, f32, f32)>,
    pub save_at: Option<(std::time::Duration, PathBuf)>,
    cg_ani: Option<Rc<eden_formats::Ani>>,
    shake_snapshot: Option<ShakeSnapshot>,
    progress: std::collections::BTreeMap<String, String>,
    cg_seen: Rc<RefCell<CgSeen>>,
    read_history: Rc<RefCell<crate::read_history::ReadHistory>>,
    pub settings: Rc<RefCell<Settings>>,
    platform_requests: VecDeque<PlatformRequest>,
    next_request_id: RequestId,
    active_context_menu: Option<RequestId>,
    pending_save_comment: Option<(RequestId, usize)>,
    game_menu: crate::gamemenu::GameMenu,
    adv_mode: u8,
    auto_ticks: u32,
    notify: Option<(String, u32)>,
    audio: Rc<RefCell<dyn crate::audio::AudioBackend>>,
    storage: Rc<dyn StorageBackend>,
    movie: Rc<RefCell<dyn VideoBackend>>,
    movie_key_armed: bool,
    left_pointer_held: bool,
    menu_press: Option<usize>,
    message_panel_hidden: bool,
    backlog: Option<BacklogView>,
    last_save_page: i32,
    screen_mode: ScreenMode,
    pending_screen_mode: Option<ScreenMode>,
    pending_exit: bool,
    start: web_time::Instant,
}
impl Engine {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        window: impl wgpu::WindowHandle + 'static,
        vfs_root: impl Into<PathBuf>,
        phys_w: u32,
        phys_h: u32,
    ) -> anyhow::Result<Self> {
        Self::new_with_override(window, vfs_root, None, phys_w, phys_h)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_with_override(
        window: impl wgpu::WindowHandle + 'static,
        vfs_root: impl Into<PathBuf>,
        override_root: Option<PathBuf>,
        phys_w: u32,
        phys_h: u32,
    ) -> anyhow::Result<Self> {
        let renderer = pollster::block_on(Renderer::new(window, phys_w, phys_h))?;
        let vfs = Rc::new(Vfs::open_with_override(vfs_root, override_root)?);
        let audio: Rc<RefCell<dyn AudioBackend>> = match CpalAudio::new(
            Rc::clone(&vfs),
        ) {
            Some(audio) => Rc::new(RefCell::new(audio)),
            None => {
                log::warn!("音频输出不可用，回退静音后端");
                Rc::new(RefCell::new(NullAudio::default()))
            }
        };
        let cg_seen = Rc::new(RefCell::new(crate::cgmode::load_cgseen()));
        let read_history = Rc::new(
            RefCell::new(
                crate::read_history::ReadHistory::load(
                    Self::save_dir().join("readflags.bin"),
                ),
            ),
        );
        let settings = Rc::new(RefCell::new(Settings::load_or_default()));
        let font = if let Some((bytes, name)) = vfs.override_font() {
            FontSource::bytes(bytes, name)
        } else {
            FontSource::System {
                preferred: settings.borrow().fontname.clone(),
            }
        };
        let storage: Rc<dyn StorageBackend> = Rc::new(
            crate::storage::FileStorage::new(Self::save_dir()),
        );
        let progress = load_progress_from(storage.as_ref());
        let video: Rc<RefCell<dyn VideoBackend>> = Rc::new(
            RefCell::new(NativeVideoBackend::new()),
        );
        Self::assemble(
            renderer,
            EngineServices {
                resources: Rc::new(
                    crate::resource::NativeResourceStore::new(Rc::clone(&vfs)),
                ),
                native_vfs: Some(vfs),
                audio,
                storage,
                video,
                font,
                settings,
                cg_seen,
                read_history,
                progress,
            },
        )
    }
    pub fn assemble(
        renderer: Renderer,
        services: EngineServices,
    ) -> anyhow::Result<Self> {
        let EngineServices {
            resources,
            native_vfs,
            audio,
            storage,
            video,
            font,
            settings,
            cg_seen,
            read_history,
            mut progress,
        } = services;
        let scene = Rc::new(RefCell::new(Scene::default()));
        let host = EngineHost::with_read_history(
            Rc::clone(&resources),
            Rc::clone(&scene),
            Rc::clone(&audio),
            Rc::clone(&cg_seen),
            Rc::clone(&settings),
            Rc::clone(&read_history),
            Rc::clone(&storage),
        );
        progress.entry("EYE_SEQ_DEFAULT".into()).or_insert_with(|| "3".into());
        let mut vm = Vm::new(
            host,
            "a001.sc",
            resources.script("a001.sc").map(|ir| (*ir).clone()).unwrap_or_default(),
        );
        vm.seed_globals(&progress);
        let atlas = font.load()?;
        let mut platform_requests = VecDeque::new();
        let start_mode = ScreenMode::from_setting(settings.borrow().window_mode);
        if let Some((x, y, width, height)) = settings.borrow().window_rect() {
            let maximized = settings.borrow().window_maximized();
            platform_requests
                .push_back(PlatformRequest::RestoreWindowState {
                    x,
                    y,
                    width,
                    height,
                    maximized,
                });
        }
        if start_mode != ScreenMode::Windowed {
            platform_requests
                .push_back(PlatformRequest::SetScreenMode {
                    mode: start_mode,
                });
        }
        Ok(Engine {
            renderer,
            resources,
            native_vfs,
            scene,
            vm: Some(vm),
            scheduler: Scheduler::new(Scheduler::DEFAULT_TPS),
            atlas,
            phase: Phase::Title {
                selection: 0,
                hover: None,
            },
            quit: false,
            fast_forward: false,
            focused: true,
            cursor: None,
            title_fade: 0,
            frame_no: 0,
            next_auto: std::time::Duration::ZERO,
            shots: Vec::new(),
            auto_advance_every: None,
            exit_after: None,
            clicks_at: Vec::new(),
            hovers_at: Vec::new(),
            save_at: None,
            cg_ani: None,
            shake_snapshot: None,
            progress,
            cg_seen,
            read_history,
            settings,
            platform_requests,
            next_request_id: 1,
            active_context_menu: None,
            pending_save_comment: None,
            game_menu: Default::default(),
            adv_mode: 0,
            auto_ticks: 0,
            notify: None,
            audio,
            storage,
            movie: video,
            movie_key_armed: false,
            left_pointer_held: false,
            menu_press: None,
            message_panel_hidden: false,
            backlog: None,
            last_save_page: 1,
            screen_mode: start_mode,
            pending_screen_mode: None,
            pending_exit: false,
            start: web_time::Instant::now(),
        })
    }
    pub fn resize(&mut self, w: u32, h: u32) {
        self.renderer.resize(w, h);
    }
    fn persist_progress(&self) {
        let bytes = match eden_formats::system_dat::encode(&self.progress) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("system.dat 编码失败: {error:?}");
                return;
            }
        };
        if let Err(error) = self.storage.write("system.dat", &bytes) {
            log::warn!("进度持久化失败: {error}");
        }
    }
    fn persistent_key(path: &Path) -> anyhow::Result<String> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!("存档路径没有有效文件名: {}", path.display())
            })?;
        Ok(format!("saves/{name}"))
    }
    fn read_persistent(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
        if self.native_vfs.is_some() {
            return Ok(std::fs::read(path)?);
        }
        self.storage
            .read(&Self::persistent_key(path)?)
            .map_err(anyhow::Error::msg)?
            .ok_or_else(|| anyhow::anyhow!("存档不存在: {}", path.display()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn write_persistent(&self, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        if self.native_vfs.is_some() {
            if let Some(directory) = path.parent() {
                std::fs::create_dir_all(directory)?;
            }
            std::fs::write(path, bytes)?;
            return Ok(());
        }
        self.storage
            .write(&Self::persistent_key(path)?, bytes)
            .map_err(anyhow::Error::msg)
    }
    pub fn mouse_move(&mut self, phys_x: f32, phys_y: f32) {
        let vp = Viewport::new(
            self.renderer.config_width(),
            self.renderer.config_height(),
        );
        let (x, y) = vp.to_logical(phys_x, phys_y);
        self.input(InputEvent::PointerMoved { x, y });
    }
    pub fn advance_time(&mut self, delta: Duration) {
        self.scheduler.advance(delta);
        let mut guard = 0u32;
        while self.scheduler.consume_one() {
            guard += 1;
            if guard > 16 {
                break;
            }
            self.tick();
        }
    }
    fn tick(&mut self) {
        self.resources.pump();
        self.audio.borrow_mut().tick();
        if self.resource_barrier_pending() {
            return;
        }
        let in_game = matches!(self.phase, Phase::Game) && self.backlog.is_none();
        if in_game {
            self.tick_movie();
        }
        if in_game {
            let allowed = self.game_menu_allowed();
            let prohibit = self.vm_prohibit_save();
            let cursor = self.cursor;
            self.game_menu.tick(allowed, cursor, prohibit);
            if self.game_menu.adv_stop {
                self.game_menu.adv_stop = false;
                self.adv_mode = 0;
                self.auto_ticks = 0;
            }
            let typing = self.scene.borrow().message.as_ref().is_some_and(|m| !m.done());
            if !typing {
                self.game_menu.corner_pause = false;
            }
            self.scene.borrow_mut().msg_type_paused = self.game_menu.corner_pause;
        }
        let background_ok = self.background_progress_allowed();
        if let (true, Some(vm)) = (in_game, &mut self.vm) {
            let control_disabled = vm.pragma_enabled("disable_control");
            let skip = (self.fast_forward || self.adv_mode == 2) && !control_disabled
                && background_ok;
            let movie_active = self.scene.borrow().movie.is_some();
            if !movie_active && skip
                && matches!(
                    vm.state(), RunState::AwaitInput | RunState::AwaitAutoMessage { .. }
                )
            {
                let typing = {
                    let mut s = self.scene.borrow_mut();
                    match &mut s.message {
                        Some(m) if !m.done() => {
                            m.reveal_all();
                            true
                        }
                        _ => false,
                    }
                };
                if !typing {
                    vm.advance_input();
                }
            }
            if skip {
                self.scene.borrow_mut().finish_scroll();
                self.scene.borrow_mut().finish_shake();
            }
            if !movie_active && self.adv_mode == 1 && background_ok
                && matches!(vm.state(), RunState::AwaitInput)
            {
                let msg_done = self
                    .scene
                    .borrow()
                    .message
                    .as_ref()
                    .is_none_or(|m| m.done());
                if msg_done {
                    self.auto_ticks += 1;
                    let need = 10
                        * self.settings.borrow().message_speed_autoplay.clamp(0, 100);
                    if self.auto_ticks as i64 >= need {
                        self.auto_ticks = 0;
                        stop_voice_on_advance(&self.settings, &self.audio);
                        vm.advance_input();
                    }
                } else {
                    self.auto_ticks = 0;
                }
            } else {
                self.auto_ticks = 0;
            }
            let _ = vm.tick();
            if matches!(vm.state(), RunState::WaitScroll)
                && self.scene.borrow().scroll.is_none()
            {
                vm.advance_scroll();
            }
            if matches!(vm.state(), RunState::WaitShake)
                && self.scene.borrow().shake.is_none()
            {
                vm.advance_shake();
            }
            let mut guard = 0u32;
            let mut chained = false;
            let mut ended = false;
            while matches!(vm.state(), RunState::Running) {
                guard += 1;
                if guard > 10_000 {
                    log::warn!("单 tick 步数超限（疑似死循环脚本）");
                    break;
                }
                match vm.step() {
                    Step::Continue | Step::Jumped | Step::Chain { .. } => chained = true,
                    Step::Ended => {
                        ended = true;
                        break;
                    }
                    _ => break,
                }
            }
            if self.adv_mode == 2
                && self
                    .scene
                    .borrow()
                    .message
                    .as_ref()
                    .is_some_and(|message| !message.is_read)
            {
                self.adv_mode = 0;
                self.auto_ticks = 0;
            }
            if chained || ended {
                self.snapshot_progress();
            }
            if ended {
                let efe = self
                    .vm
                    .as_ref()
                    .and_then(|v| {
                        v.locals()
                            .get("exitFromEvent")
                            .or_else(|| v.globals().get("exitFromEvent"))
                            .cloned()
                    })
                    .unwrap_or_default();
                if efe == "999" {
                    log::info!("脚本终止 → Memories（exitFromEvent=999）");
                    self.progress.remove("exitFromEvent");
                    self.persist_progress();
                    self.audio.borrow_mut().stop_all();
                    self.phase = Phase::Memories { hover: None };
                    *self.scene.borrow_mut() = Scene::default();
                    self.game_menu = Default::default();
                    self.restart_vm("a001.sc");
                } else {
                    log::info!("脚本终止 → 回标题（exitFromEvent={efe}）");
                    self.to_title();
                }
            }
        }
        self.title_fade = self.title_fade.saturating_add(10);
        let mut save_comment_request = None;
        let mut sl_finish: Option<(u8, i32, Option<usize>, Option<String>, bool)> = None;
        if let Phase::SaveLoad(menu) = &mut self.phase {
            menu.tick();
            save_comment_request = menu.take_save_comment_request();
            if menu.dead {
                sl_finish = Some((
                    menu.mode,
                    menu.page as i32,
                    menu.pending,
                    menu.pending_comment.take(),
                    menu.from_game,
                ));
            }
        }
        if let Some(abs) = save_comment_request {
            let path = crate::saveload::slot_file(abs, "sav");
            let initial = self
                .read_persistent(&path)
                .ok()
                .and_then(|b| eden_formats::read_save_file(&b).ok())
                .map(|f| f.header.comment)
                .unwrap_or_default();
            let request_id = self.next_platform_request_id();
            self.pending_save_comment = Some((request_id, abs));
            self.platform_requests
                .push_back(PlatformRequest::EditSaveComment {
                    request_id,
                    initial,
                    max_chars: 36,
                });
        }
        if let Some((mode, page, pending, pending_comment, from_game)) = sl_finish {
            self.last_save_page = page;
            if from_game {
                self.phase = Phase::Game;
            } else {
                self.to_title();
            }
            if let Some(abs) = pending {
                let path = crate::saveload::slot_file(abs, "sav");
                if mode == 0 {
                    let comment = pending_comment.unwrap_or_default();
                    if let Err(err) = self.save_to(&path, &comment) {
                        log::error!("存档失败 {}: {err:?}", path.display());
                    }
                } else if let Err(err) = self.load(&path) {
                    log::error!("读档失败 {}: {err:?}", path.display());
                    self.to_title();
                }
            }
        }
        let mut cfg_dead = false;
        if let Phase::Config(cfg, _) = &mut self.phase {
            cfg.tick();
            cfg_dead = cfg.dead;
        }
        if cfg_dead {
            let restore_game_audio = match &self.phase {
                Phase::Config(_, prev) => matches!(prev.as_ref(), Phase::Game),
                _ => false,
            };
            let prev = match std::mem::replace(&mut self.phase, Phase::Game) {
                Phase::Config(_, prev) => *prev,
                other => other,
            };
            self.phase = prev;
            if restore_game_audio {
                self.restore_audio_after_config();
            }
        }
        if let Some((_, t)) = &mut self.notify {
            *t = t.saturating_sub(1);
            if *t == 0 {
                self.notify = None;
            }
        }
        self.tick_cg_anim();
        self.scene.borrow_mut().effect_tick = self.settings.borrow().screen_effect != 0;
        self.scene.borrow_mut().tick();
        let mut gallery_dead = false;
        if let Phase::CgGallery(g) = &mut self.phase {
            if let Some(v) = &mut g.viewer {
                if v.tick() {
                    g.viewer = None;
                }
            } else {
                g.tick();
                gallery_dead = g.dead;
            }
        }
        if gallery_dead {
            self.to_memories();
        }
        let mut music_dead = false;
        if let Phase::MusicMenu(m) = &mut self.phase {
            m.tick();
            music_dead = m.dead;
            if !music_dead && m.playing {
                if let Some(t) = self.audio.borrow().bgm_elapsed_secs() {
                    m.elapsed = t.max(0.0) as u32;
                }
                if !self.audio.borrow().bgm_playing() {
                    let next = m.advance();
                    let mut audio = self.audio.borrow_mut();
                    match next {
                        Some(name) => {
                            audio.play_bgm(name, 100, 0, false, None, None, false)
                        }
                        None => {
                            audio.stop_bgm(None);
                            m.playing = false;
                        }
                    }
                } else {
                    self.audio.borrow().spectrum(&mut m.spectrum);
                }
            }
        }
        if music_dead {
            self.to_memories();
        }
    }
    fn resource_barrier_pending(&self) -> bool {
        let mut names = Vec::<String>::new();
        match &self.phase {
            Phase::Title { hover, .. } => {
                let index = title_bg_index(
                    global_flag(&self.progress, "eden_D001"),
                    global_flag(&self.progress, "eden_B006"),
                );
                names.push(format!("topmenu{index}.png"));
                if hover.is_some() {
                    names.push(format!("topmenu{index}over.png"));
                }
            }
            Phase::Game => {
                let scene = self.scene.borrow();
                names.extend(scene.bg.iter().map(|bg| bg.name.clone()));
                names.extend(scene.prev_bg.iter().map(|bg| bg.name.clone()));
                if let Some(mask) = scene
                    .active_tr
                    .as_ref()
                    .and_then(|transition| transition.spec.mask.as_ref())
                {
                    names.push(mask.clone());
                }
                if !self.message_panel_hidden {
                    if let Some(transition) = &scene.panel_tr {
                        for mode in [transition.from, transition.to] {
                            let name = panel_asset(mode);
                            if !name.is_empty() {
                                names.push(name.into());
                            }
                        }
                    } else {
                        let name = panel_asset(scene.panel_mode);
                        if !name.is_empty() {
                            names.push(name.into());
                        }
                    }
                }
            }
            _ => {}
        }
        names
            .into_iter()
            .any(|name| {
                matches!(
                    self.resources.request_image(& name), crate
                    ::resource::ResourceState::Pending
                )
            })
    }
    fn tick_movie(&mut self) {
        let pending = {
            let scene = self.scene.borrow();
            match &scene.movie {
                Some(crate::scene::MovieState::Pending(request)) => Some(request.clone()),
                _ => None,
            }
        };
        if let Some(request) = pending {
            let source = self
                .native_vfs
                .as_ref()
                .map(|vfs| vfs.movie_source(&request.file))
                .unwrap_or_else(|| crate::vfs::MovieSource::Unavailable(
                    request.file.clone(),
                ));
            let target = MovieTarget {
                x: request.x,
                y: request.y,
                width: request.width,
                height: request.height,
            };
            self.movie.borrow_mut().play(source, target, &mut *self.audio.borrow_mut());
            self.scene.borrow_mut().movie = Some(
                crate::scene::MovieState::Playing(request),
            );
        }
        if self.scene.borrow().movie.is_none() {
            return;
        }
        let volume = {
            let settings = self.settings.borrow();
            if settings.mute_bgm { 0 } else { settings.volume_bgm }
        };
        let status = self.movie.borrow_mut().poll(&mut *self.audio.borrow_mut(), volume);
        if matches!(status, MovieStatus::Finished | MovieStatus::Failed) {
            self.finish_movie_wait();
        }
    }
    fn stop_movie_playback(&mut self) {
        self.movie.borrow_mut().cleanup(&mut *self.audio.borrow_mut());
        self.scene.borrow_mut().movie = None;
    }
    fn finish_movie_wait(&mut self) {
        self.stop_movie_playback();
        if let Some(vm) = &mut self.vm {
            if matches!(vm.state(), RunState::AwaitInput) {
                vm.advance_input();
            }
        }
    }
    fn tick_cg_anim(&mut self) {
        let (bg_name, need_build) = {
            let s = self.scene.borrow();
            let name = s.bg.as_ref().map(|b| b.name.to_ascii_lowercase());
            (name.clone(), name.is_some() && s.cg_anim.is_none())
        };
        if need_build {
            let name = bg_name.unwrap();
            let ani = self.native_vfs.as_ref().and_then(|vfs| vfs.ani(&name));
            let anim = ani
                .as_ref()
                .and_then(|ani| {
                    let mut chars: Vec<String> = Vec::new();
                    for f in &ani.frames {
                        if let Some(nm) = f.label.split(' ').next() {
                            if nm != f.label && !chars.iter().any(|c| c == nm) {
                                chars.push(nm.to_string());
                            }
                        }
                    }
                    (!chars.is_empty()).then(|| crate::scene::CgAnim::new(&name, chars))
                });
            self.scene.borrow_mut().cg_anim = anim;
            self.cg_ani = ani;
        }
        let (voice, speaker) = {
            let s = self.scene.borrow();
            let v = s
                .message
                .as_ref()
                .map(|m| (m.voice.is_some(), 0u32))
                .unwrap_or((false, 0));
            (v, s.message.as_ref().and_then(|m| m.speaker.clone()))
        };
        let (has_voice, _) = voice;
        let level = (self.audio.borrow().voice_level() * 1000.0) as u32;
        let voice = (has_voice, level);
        if self.settings.borrow().animation != 0 {
            if let Some(anim) = &mut self.scene.borrow_mut().cg_anim {
                anim.tick(speaker.as_deref(), voice);
            }
        }
    }
    fn to_title(&mut self) {
        self.stop_movie_playback();
        self.menu_press = None;
        self.audio.borrow_mut().stop_all();
        self.backlog = None;
        self.message_panel_hidden = false;
        self.active_context_menu = None;
        self.pending_save_comment = None;
        self.phase = Phase::Title {
            selection: 0,
            hover: None,
        };
        *self.scene.borrow_mut() = Scene::default();
        self.restart_vm("a001.sc");
    }
    fn memories_activate(&mut self, item: usize) {
        self.menu_press = None;
        match MEMORIES_MENU[item].0 {
            "Snaps" => {
                self.audio.borrow_mut().stop_all();
                self.phase = Phase::CgGallery(Box::new(CgGallery::open(&self.cg_seen)));
            }
            "Music" => {
                self.audio.borrow_mut().stop_all();
                self.phase = Phase::MusicMenu(Box::new(MusicMenu::open()));
            }
            "Movie" => {
                self.phase = Phase::Game;
                *self.scene.borrow_mut() = Scene::default();
                self.restart_vm("memOp.sc");
                if self.vm.is_none() {
                    log::error!("memOp.sc 缺失");
                    self.to_title();
                }
            }
            "Return" => self.to_title(),
            other => {
                log::info!(
                    "Memories {other}：Scene 外传门控（资产缺失，恒锁）"
                )
            }
        }
    }
    fn to_memories(&mut self) {
        self.stop_movie_playback();
        self.menu_press = None;
        self.audio.borrow_mut().stop_all();
        self.phase = Phase::Memories { hover: None };
    }
    fn restart_vm(&mut self, script: &str) {
        let key = script.to_ascii_lowercase();
        let Some(ir) = self.resources.script(&key) else {
            log::error!("脚本缺失: {key}");
            return;
        };
        let host = EngineHost::with_read_history(
            Rc::clone(&self.resources),
            Rc::clone(&self.scene),
            Rc::clone(&self.audio),
            Rc::clone(&self.cg_seen),
            Rc::clone(&self.settings),
            Rc::clone(&self.read_history),
            Rc::clone(&self.storage),
        );
        let mut vm = Vm::new(host, key, (*ir).clone());
        vm.seed_globals(&self.progress);
        self.vm = Some(vm);
    }
    pub fn start_new_game(&mut self) {
        self.phase = Phase::Game;
        self.backlog = None;
        self.message_panel_hidden = false;
        self.active_context_menu = None;
        self.pending_save_comment = None;
        *self.scene.borrow_mut() = Scene::default();
        self.game_menu = Default::default();
        self.adv_mode = 0;
        self.auto_ticks = 0;
        self.restart_vm("test.sc");
        if self.vm.is_none() {
            self.restart_vm("a001.sc");
        }
    }
    fn title_item_enabled(&self, item: usize) -> bool {
        title_item_enabled(&self.progress, item)
    }
    fn snapshot_progress(&mut self) {
        if let Some(vm) = &self.vm {
            self.progress = vm.globals().clone();
        }
        self.persist_progress();
    }
    fn title_activate(&mut self, item: usize) {
        self.menu_press = None;
        match TITLE_MENU[item].0 {
            "NewGame" => self.start_new_game(),
            "LoadGame" => self.open_save_load(1),
            "System" => self.open_config(),
            "Memories" => {
                self.phase = Phase::Memories { hover: None };
            }
            "Exit" => self.request_exit(),
            other => {
                log::info!(
                    "标题菜单 {other}：Mosaic 属外传项（本篇无资产恒不可达）"
                )
            }
        }
    }
    fn open_config(&mut self) {
        let cfg = crate::config::ConfigState::new(
            &self.settings.borrow(),
            self.atlas.face_name(),
        );
        let prev = std::mem::replace(&mut self.phase, Phase::Game);
        self.phase = Phase::Config(Box::new(cfg), Box::new(prev));
    }
    fn restore_audio_after_config(&mut self) {
        let Some(vm) = &self.vm else { return };
        let snapshot = vm.system_snapshot();
        self.restore_saved_loop_audio(&snapshot);
    }
    fn restore_saved_loop_audio(
        &mut self,
        snapshot: &std::collections::BTreeMap<String, String>,
    ) {
        let s = self.settings.borrow().clone();
        let mut audio = self.audio.borrow_mut();
        Self::restore_loop_audio_backend(&mut *audio, snapshot, &s);
    }
    fn restore_loop_audio_backend(
        audio: &mut dyn crate::audio::AudioBackend,
        snapshot: &std::collections::BTreeMap<String, String>,
        s: &Settings,
    ) {
        if let Some(name) = snapshot
            .get("bgm_Filename")
            .filter(|name| !name.is_empty() && name.as_str() != "*")
        {
            let script_volume = snapshot
                .get("bgm_Volume")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(100);
            let (resolved, _, pan) = parse_vol_pan(name, 100);
            audio
                .play_bgm(
                    &resolved,
                    script_volume * s.volume_bgm / 100,
                    pan,
                    true,
                    None,
                    None,
                    s.mute_bgm,
                );
        } else {
            audio.stop_bgm(None);
        }
        for (key, se2) in [("se1_Filename", false), ("se2_Filename", true)] {
            if let Some(name) = snapshot
                .get(key)
                .filter(|name| !name.is_empty() && name.as_str() != "*")
            {
                let (resolved, volume, pan) = parse_vol_pan(name, s.volume_se);
                audio
                    .play_se(
                        &resolved,
                        volume,
                        pan,
                        se2,
                        true,
                        Some(1),
                        Some(1),
                        s.mute_se,
                    );
            } else {
                audio.stop_se(se2, None);
            }
        }
    }
    fn game_advance_click(&mut self) {
        if self.vm.as_ref().is_some_and(|vm| vm.pragma_enabled("disable_control")) {
            return;
        }
        if self.scene.borrow().movie.is_some() {
            return;
        }
        let (scrolling, shaking, shake_click_skippable, panel_fading) = {
            let scene = self.scene.borrow();
            (
                scene.scroll.is_some(),
                scene.shake.is_some(),
                scene.shake_click_skippable(),
                scene.panel_tr.is_some(),
            )
        };
        if scrolling {
            let mut scene = self.scene.borrow_mut();
            scene.finish_scroll();
            drop(scene);
            if let Some(vm) = &mut self.vm {
                if matches!(vm.state(), RunState::WaitScroll) {
                    vm.advance_scroll();
                }
            }
            return;
        }
        if shaking {
            if shake_click_skippable {
                self.scene.borrow_mut().finish_shake();
                if let Some(vm) = &mut self.vm {
                    if matches!(vm.state(), RunState::WaitShake) {
                        vm.advance_shake();
                    }
                }
            }
            return;
        }
        if panel_fading {
            self.scene.borrow_mut().finish_panel_transition();
            if let Some(vm) = &mut self.vm {
                if matches!(vm.state(), RunState::WaitFrames(_)) {
                    vm.pump(u32::MAX);
                }
            }
            return;
        }
        let mut scene = self.scene.borrow_mut();
        if let Some(msg) = &mut scene.message {
            if !msg.done() {
                msg.reveal_all();
                return;
            }
        }
        drop(scene);
        self.stop_voice_for_advance();
        if let Some(vm) = &mut self.vm {
            vm.advance_input();
        }
    }
    fn gamemenu_click(&mut self, btn: usize) {
        {
            let s = self.settings.borrow();
            if !s.mute_se {
                let v = s.volume_se;
                drop(s);
                self.audio.borrow_mut().play_aux("click.wav", v, AuxChannel::Ui);
            }
        }
        match btn {
            0 => {
                self.adv_mode = if self.settings.borrow().play_mode == 0 {
                    2
                } else {
                    1
                };
                self.auto_ticks = 0;
            }
            1 => {
                let pc = self.vm.as_ref().map_or(u32::MAX, |v| v.pc());
                if self.game_menu.last_qsave_pc != Some(pc) {
                    self.game_menu.last_qsave_pc = Some(pc);
                    self.adv_mode = 0;
                    let n = self.settings.borrow().quick_save_file_number.clamp(1, 100);
                    let path = Self::save_dir().join(format!("edenPM{n:04}.sav"));
                    match self.quick_save(&path) {
                        Ok(()) => {
                            self.settings.borrow_mut().quick_save_file_number = if n
                                >= 100
                            {
                                1
                            } else {
                                n + 1
                            };
                            self.notify = Some(("nmQuickSave.png".into(), 50));
                        }
                        Err(e) => log::error!("快存失败: {e:?}"),
                    }
                }
            }
            2 => {
                self.adv_mode = 0;
                {
                    let mut a = self.audio.borrow_mut();
                    a.stop_bgm(None);
                    a.stop_se(false, None);
                    a.stop_se(true, None);
                    a.stop_voice();
                }
                self.open_config();
            }
            3 => {
                self.adv_mode = 0;
                self.open_save_load(1);
            }
            4 => {
                self.adv_mode = 0;
                self.open_save_load(0);
            }
            _ => {}
        }
    }
    fn game_menu_allowed(&self) -> bool {
        {
            let scene = self.scene.borrow();
            if scene.movie.is_some() || scene.shake.is_some()
                || scene.active_tr.is_some()
            {
                return false;
            }
        }
        let disable = self
            .vm
            .as_ref()
            .is_some_and(|v| v.pragma_enabled("disable_control"));
        if disable {
            return false;
        }
        match self.vm.as_ref().map(|v| v.state()) {
            Some(RunState::Running) | None => true,
            Some(RunState::WaitFrames(_))
            | Some(RunState::AwaitAutoMessage { .. })
            | Some(RunState::WaitShake)
            | Some(RunState::WaitScroll) => false,
            Some(RunState::AwaitInput) => {
                self.scene.borrow().message.as_ref().is_none_or(|m| m.done())
            }
            Some(RunState::Ended) => false,
        }
    }
    fn vm_prohibit_save(&self) -> bool {
        let Some(vm) = &self.vm else { return false };
        let v = vm
            .locals()
            .get("prohibitSaveLoad")
            .or_else(|| vm.globals().get("prohibitSaveLoad"));
        v.is_some_and(|s| !s.is_empty() && s != "0")
    }
    pub(crate) fn save_dir() -> std::path::PathBuf {
        SAVE_DIR.get_or_init(resolve_save_dir).clone()
    }
    fn open_save_load(&mut self, mode: u8) {
        let page = self.last_save_page;
        let from_game = matches!(self.phase, Phase::Game);
        let menu = if self.native_vfs.is_some() {
            SaveLoadMenu::open(mode, page, from_game)
        } else {
            SaveLoadMenu::open_with_storage(
                mode,
                page,
                from_game,
                Rc::clone(&self.storage),
            )
        };
        self.phase = Phase::SaveLoad(Box::new(menu));
    }
    pub fn drain_platform_requests(&mut self) -> Vec<PlatformRequest> {
        self.platform_requests.drain(..).collect()
    }
    pub fn invalidate_resource_image(&self, name: &str) {
        self.resources.invalidate_image(name);
    }
    pub fn platform_response(&mut self, response: PlatformResponse) {
        match response {
            PlatformResponse::ScreenModeApplied { mode, ok } => {
                if self.pending_screen_mode != Some(mode) {
                    return;
                }
                self.pending_screen_mode = None;
                if ok {
                    self.screen_mode = mode;
                    self.settings.borrow_mut().screen_mode = mode.as_setting();
                    persist_settings_to(self.storage.as_ref(), &self.settings.borrow());
                } else {
                    log::warn!("画面模式 {mode:?} 应用失败，保持原模式");
                }
            }
            PlatformResponse::WindowGeometry(geometry) => {
                self.save_window_geometry(geometry);
                if self.pending_exit {
                    self.pending_exit = false;
                    self.quit = true;
                }
            }
            PlatformResponse::ContextMenuClosed {
                request_id,
                command,
            } if self.active_context_menu == Some(request_id) => {
                self.active_context_menu = None;
                if let Some(command) = command {
                    self.context_menu_command(command);
                }
            }
            PlatformResponse::SaveCommentClosed { request_id, comment } => {
                let Some((pending_id, abs)) = self.pending_save_comment.take() else {
                    return;
                };
                if pending_id != request_id {
                    self.pending_save_comment = Some((pending_id, abs));
                    return;
                }
                if let Phase::SaveLoad(menu) = &mut self.phase {
                    let _ = menu.resolve_save_comment(abs, comment);
                }
            }
            _ => {}
        }
    }
    pub fn input(&mut self, event: InputEvent) {
        match event {
            InputEvent::PointerMoved { x, y } => {
                self.handle_action(Action::MouseMove { x, y })
            }
            InputEvent::PointerLeft => self.handle_action(Action::MouseLeft),
            InputEvent::WheelSteps { x, y } => {
                self.handle_action(Action::MouseWheel { dx: x, dy: y })
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ButtonState::Pressed,
                click_count,
            } => {
                self.left_pointer_held = true;
                if self.scene.borrow().movie.is_none() {
                    self.handle_action(Action::Click {
                        click_count: click_count.max(1),
                    });
                }
            }
            InputEvent::PointerButton {
                button: PointerButton::Left,
                state: ButtonState::Released,
                ..
            } => {
                self.left_pointer_held = false;
                if !self.finish_movie_from_release() {
                    self.handle_action(Action::MouseRelease);
                }
            }
            InputEvent::PointerButton {
                button: PointerButton::Right,
                state: ButtonState::Released,
                ..
            } => {
                if !self.finish_movie_from_release() {
                    self.handle_action(Action::RightClick);
                }
            }
            InputEvent::PointerButton {
                button: PointerButton::Middle,
                state: ButtonState::Released,
                ..
            } => self.handle_action(Action::MiddleRelease),
            InputEvent::PointerButton { .. } => {}
            InputEvent::Key { key, state: ButtonState::Pressed, repeat } => {
                self.key_pressed(key, repeat)
            }
            InputEvent::Key { key, state: ButtonState::Released, .. } => {
                self.key_released(key)
            }
        }
    }
    fn handle_action(&mut self, input: Action) {
        if self.backlog.is_some() && matches!(self.phase, Phase::Game) {
            self.handle_backlog_action(input);
            return;
        }
        if matches!(input, Action::MouseLeft) {
            self.pointer_left();
            return;
        }
        let enabled: Vec<bool> = (0..TITLE_MENU.len())
            .map(|i| self.title_item_enabled(i))
            .collect();
        match (&mut self.phase, input) {
            (Phase::Title { selection, hover }, Action::Up) => {
                loop {
                    *selection = selection
                        .checked_sub(1)
                        .unwrap_or(visible_title_items() - 1);
                    if enabled[*selection] {
                        break;
                    }
                }
                *hover = Some(*selection);
            }
            (Phase::Title { selection, hover }, Action::Down) => {
                loop {
                    *selection = (*selection + 1) % TITLE_MENU.len();
                    if enabled[*selection] {
                        break;
                    }
                }
                *hover = Some(*selection);
            }
            (Phase::Title { selection, hover }, Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                *hover = title_hit_test(x, y).filter(|&i| enabled[i]);
                if let Some(h) = *hover {
                    *selection = h;
                }
            }
            (Phase::Title { selection, hover }, Action::Click { .. }) => {
                self.menu_press = self
                    .cursor
                    .and_then(|(x, y)| title_hit_test(x, y))
                    .filter(|&i| enabled[i]);
                if let Some(i) = self.menu_press {
                    *hover = Some(i);
                    *selection = i;
                }
            }
            (Phase::Title { selection, hover }, Action::MouseRelease) => {
                let pressed = self.menu_press.take();
                let released = self
                    .cursor
                    .and_then(|(x, y)| title_hit_test(x, y))
                    .filter(|&i| enabled[i]);
                if let Some(i) = menu_release_commit(pressed, released) {
                    *hover = Some(i);
                    *selection = i;
                    self.title_activate(i);
                }
            }
            (Phase::Title { selection, hover }, Action::Advance) => {
                let sel = *selection;
                if enabled[sel] {
                    *hover = Some(sel);
                    self.title_activate(sel);
                }
            }
            (Phase::Title { .. }, Action::Cancel) => {
                queue_exit(
                    &mut self.quit,
                    &mut self.pending_exit,
                    &mut self.platform_requests,
                );
            }
            (Phase::SaveLoad(menu), Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                let _ = menu;
            }
            (Phase::SaveLoad(menu), Action::Click { click_count }) => {
                if menu.is_active() {
                    if let Some((x, y)) = self.cursor {
                        let cmd = crate::saveload::hit_cmd(x, y);
                        menu.pointer_press(cmd, click_count);
                    }
                }
            }
            (Phase::SaveLoad(menu), Action::MouseRelease) => {
                if menu.is_active() {
                    let cmd = self
                        .cursor
                        .map(|(x, y)| crate::saveload::hit_cmd(x, y))
                        .unwrap_or(0);
                    menu.pointer_release(cmd);
                }
            }
            (Phase::SaveLoad(menu), Action::Advance) => {
                if menu.is_active() {
                    menu.confirm();
                }
            }
            (Phase::SaveLoad(menu), Action::Cancel) => {
                if menu.is_active() {
                    menu.close();
                }
            }
            (Phase::SaveLoad(menu), Action::Up) => {
                if menu.is_active() {
                    menu.key_up()
                }
            }
            (Phase::SaveLoad(menu), Action::Down) => {
                if menu.is_active() {
                    menu.key_down()
                }
            }
            (Phase::SaveLoad(menu), Action::Left) => {
                if menu.is_active() {
                    menu.key_left()
                }
            }
            (Phase::SaveLoad(menu), Action::Right) => {
                if menu.is_active() {
                    menu.key_right()
                }
            }
            (Phase::SaveLoad(menu), Action::PageUp) => {
                if menu.is_active() {
                    menu.turn_page(-1)
                }
            }
            (Phase::SaveLoad(menu), Action::PageDown) => {
                if menu.is_active() {
                    menu.turn_page(1)
                }
            }
            (Phase::SaveLoad(menu), Action::HomeKey) => {
                if menu.is_active() {
                    let first = if menu.mode == 0 { 1 } else { 0 };
                    menu.jump_page(first);
                }
            }
            (Phase::SaveLoad(menu), Action::EndKey) => {
                if menu.is_active() {
                    menu.jump_page(9)
                }
            }
            (Phase::Memories { hover }, Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                *hover = memories_hit_test(x, y);
            }
            (Phase::Memories { hover }, Action::Click { .. }) => {
                self.menu_press = self
                    .cursor
                    .and_then(|(x, y)| memories_hit_test(x, y))
                    .or(*hover);
                if let Some(i) = self.menu_press {
                    *hover = Some(i);
                }
            }
            (Phase::Memories { hover }, Action::MouseRelease) => {
                let pressed = self.menu_press.take();
                let released = self
                    .cursor
                    .and_then(|(x, y)| memories_hit_test(x, y))
                    .or(*hover);
                if let Some(i) = menu_release_commit(pressed, released) {
                    self.memories_activate(i);
                }
            }
            (Phase::Memories { .. }, Action::Cancel | Action::Advance) => {
                self.to_title();
            }
            (Phase::CgGallery(g), Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                let _ = g;
            }
            (Phase::CgGallery(g), Action::Click { .. }) => {
                let (x, y) = match self.cursor {
                    Some(p) => p,
                    None => return,
                };
                if g.viewer.is_some() {
                    if let Some(v) = &mut g.viewer {
                        v.advance();
                    }
                    viewer_skip_missing(self.resources.as_ref(), g);
                    return;
                }
                if g.fade < 255 {
                    return;
                }
                for cell in 0..25usize {
                    let (cx, cy) = (
                        crate::cgmode::GRID_XS[cell % 5],
                        crate::cgmode::GRID_YS[cell / 5],
                    );
                    if (cx..cx + crate::cgmode::CELL_W).contains(&x)
                        && (cy..cy + crate::cgmode::CELL_H).contains(&y)
                    {
                        if let Some(entry) = g.cell_entry(cell) {
                            if self
                                .cg_seen
                                .borrow()
                                .contains(crate::cgmode::CgGallery::display_name(entry))
                            {
                                let names = g.viewer_names(cell);
                                g.viewer = Some(CgViewer::new(names));
                                viewer_skip_missing(self.resources.as_ref(), g);
                            }
                        }
                        return;
                    }
                }
                let r = crate::cgmode::BTN_BACK;
                if in_rect(x, y, r) {
                    g.command(2);
                    return;
                }
                let r = crate::cgmode::BTN_NEXT;
                if in_rect(x, y, r) && g.pages >= 2 {
                    g.command(3);
                    return;
                }
                let r = crate::cgmode::BTN_RETURN;
                if in_rect(x, y, r) {
                    g.command(4);
                    return;
                }
            }
            (Phase::CgGallery(g), Action::Advance) => {
                if g.viewer.is_some() {
                    if let Some(v) = &mut g.viewer {
                        v.advance();
                    }
                    viewer_skip_missing(self.resources.as_ref(), g);
                }
            }
            (Phase::CgGallery(g), Action::Cancel) => {
                if g.viewer.take().is_none() {
                    g.close();
                }
            }
            (Phase::CgGallery(g), Action::PageUp) => g.command(2),
            (Phase::CgGallery(g), Action::PageDown) => g.command(3),
            (Phase::CgGallery(g), Action::MouseWheel { dx, dy }) => {
                if g.viewer.as_ref().is_none_or(|v| v.fade < 50) {
                    return;
                }
                let (sx, sy) = gallery_wheel_steps(dx, dy);
                let wide = g
                    .viewer
                    .as_ref()
                    .and_then(|v| v.current())
                    .and_then(|n| self.resources.request_image(n).ready())
                    .is_some_and(|i| i.logical_w > 1024.0);
                if wide && sx == 0 {
                    gallery_scroll(self.resources.as_ref(), g, sy, 0);
                } else {
                    gallery_scroll(self.resources.as_ref(), g, sx, sy);
                }
            }
            (Phase::CgGallery(g), Action::Up) => {
                gallery_scroll(self.resources.as_ref(), g, 0, -200)
            }
            (Phase::CgGallery(g), Action::Down) => {
                gallery_scroll(self.resources.as_ref(), g, 0, 200)
            }
            (Phase::CgGallery(g), Action::Left) => {
                gallery_scroll(self.resources.as_ref(), g, -200, 0)
            }
            (Phase::CgGallery(g), Action::Right) => {
                gallery_scroll(self.resources.as_ref(), g, 200, 0)
            }
            (Phase::MusicMenu(m), Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                let _ = m;
            }
            (Phase::MusicMenu(m), Action::Click { click_count }) => {
                let (x, y) = match self.cursor {
                    Some(p) => p,
                    None => return,
                };
                if m.fade < 255 {
                    return;
                }
                use crate::music as mus;
                if in_rect(x, y, mus::BTN_PREV) {
                    if let Some(name) = m.step(-1) {
                        self.audio
                            .borrow_mut()
                            .play_bgm(name, 100, 0, false, None, None, false);
                    }
                    return;
                }
                if in_rect(x, y, mus::BTN_PLAY) {
                    if m.selected >= 0 {
                        if let Some(name) = m.play(m.selected) {
                            self.audio
                                .borrow_mut()
                                .play_bgm(name, 100, 0, false, None, None, false);
                        }
                    }
                    return;
                }
                if in_rect(x, y, mus::BTN_STOP) {
                    self.audio.borrow_mut().stop_bgm(None);
                    m.stop();
                    return;
                }
                if in_rect(x, y, mus::BTN_NEXT) {
                    if let Some(name) = m.step(1) {
                        self.audio
                            .borrow_mut()
                            .play_bgm(name, 100, 0, false, None, None, false);
                    }
                    return;
                }
                if in_rect(x, y, mus::BTN_MODE) {
                    m.toggle_mode();
                    return;
                }
                if in_rect(x, y, mus::BTN_PREVPAGE) {
                    m.turn_mode(-1);
                    self.audio.borrow_mut().stop_bgm(None);
                    return;
                }
                if in_rect(x, y, mus::BTN_NEXTPAGE) {
                    m.turn_mode(1);
                    self.audio.borrow_mut().stop_bgm(None);
                    return;
                }
                if in_rect(x, y, mus::BTN_EXIT) {
                    m.close();
                    return;
                }
                let r = mus::TRACK_HIT;
                if in_rect(x, y, r) {
                    let row = ((y - r[1]) / mus::ROW_H) as i32;
                    if m.row_track(row).is_some() {
                        if click_count >= 2 {
                            if let Some(name) = m.play(row) {
                                self.audio
                                    .borrow_mut()
                                    .play_bgm(name, 100, 0, false, None, None, false);
                            }
                        } else {
                            m.selected = row;
                            if !m.playing {
                                m.highlight = row;
                            }
                        }
                    }
                }
            }
            (Phase::MusicMenu(m), Action::Advance) => {
                if m.selected >= 0 {
                    if let Some(name) = m.play(m.selected) {
                        self.audio
                            .borrow_mut()
                            .play_bgm(name, 100, 0, false, None, None, false);
                    }
                }
            }
            (Phase::MusicMenu(m), Action::Cancel) => m.close(),
            (Phase::Config(cfg, _), Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                cfg.hover = crate::config::hit(x, y)
                    .filter(|cmd| matches!(* cmd, 100 | 101));
                if let (true, Some(cmd)) = (cfg.is_active(), cfg.drag) {
                    config_slider_set(
                        &self.settings,
                        cmd,
                        crate::config::slider_value(cmd, x),
                    );
                }
            }
            (Phase::Config(cfg, _), Action::Click { .. }) => {
                if !cfg.is_active() {
                    return;
                }
                let Some((x, y)) = self.cursor else { return };
                let Some(cmd) = crate::config::hit(x, y) else {
                    return;
                };
                cfg.hover = matches!(cmd, 100 | 101).then_some(cmd);
                cfg.pressed_button = None;
                match cmd {
                    1..=3 | 17..=19 => {
                        cfg.drag = Some(cmd);
                        self.platform_requests
                            .push_back(PlatformRequest::SetPointerCapture {
                                enabled: true,
                            });
                        config_slider_set(
                            &self.settings,
                            cmd,
                            crate::config::slider_value(cmd, x),
                        );
                    }
                    8 => cfg.screen_mode = 1,
                    9 => cfg.screen_mode = 0,
                    100 => {
                        cfg.pressed_button = Some(100);
                    }
                    101 => {
                        cfg.pressed_button = Some(101);
                    }
                    4 | 5 => {
                        let direction = if cmd == 4 { -1 } else { 1 };
                        match self.atlas.cycle_face(direction) {
                            Ok(face) => {
                                cfg.font_name = face.clone();
                                self.settings.borrow_mut().fontname = face;
                            }
                            Err(err) => log::warn!("字体切换失败: {err:#}"),
                        }
                    }
                    _ => config_widget(&self.settings, &self.audio, cmd),
                }
            }
            (Phase::Config(cfg, _), Action::MouseRelease) => {
                let hit = self
                    .cursor
                    .and_then(|(x, y)| crate::config::hit(x, y))
                    .filter(|cmd| matches!(* cmd, 100 | 101));
                cfg.hover = hit;
                if cfg.drag.take().is_some() {
                    self.platform_requests
                        .push_back(PlatformRequest::SetPointerCapture {
                            enabled: false,
                        });
                }
                let pressed = cfg.pressed_button.take();
                if cfg.is_active() && pressed == hit {
                    match pressed {
                        Some(100) => {
                            queue_config_screen_mode(
                                self.screen_mode,
                                &mut self.pending_screen_mode,
                                &mut self.platform_requests,
                                cfg.screen_mode != 0,
                            );
                            persist_settings_to(
                                self.storage.as_ref(),
                                &self.settings.borrow(),
                            );
                            cfg.begin_close();
                        }
                        Some(101) => {
                            if let Err(err) = self
                                .atlas
                                .set_face(&cfg.original_font_name)
                            {
                                log::warn!("恢复字体失败: {err:#}");
                            }
                            *self.settings.borrow_mut() = Settings::load_or_default();
                            cfg.begin_close();
                        }
                        _ => {}
                    }
                }
            }
            (Phase::Config(cfg, _), Action::Advance) => {
                if cfg.is_active() {
                    queue_config_screen_mode(
                        self.screen_mode,
                        &mut self.pending_screen_mode,
                        &mut self.platform_requests,
                        cfg.screen_mode != 0,
                    );
                    persist_settings_to(self.storage.as_ref(), &self.settings.borrow());
                    cfg.begin_close();
                }
            }
            (Phase::Config(cfg, _), Action::Cancel) => {
                if cfg.is_active() {
                    if let Err(err) = self.atlas.set_face(&cfg.original_font_name) {
                        log::warn!("恢复字体失败: {err:#}");
                    }
                    *self.settings.borrow_mut() = Settings::load_or_default();
                    cfg.begin_close();
                }
            }
            (Phase::Game, Action::Click { .. }) => {
                let (x, y) = self.cursor.unwrap_or((-1.0, -1.0));
                let allowed = self.game_menu_allowed();
                let prohibit = self.vm_prohibit_save();
                match self.game_menu.mouse(x, y, true, allowed, prohibit) {
                    crate::gamemenu::MouseOutcome::Command(i) => self.gamemenu_click(i),
                    crate::gamemenu::MouseOutcome::Consumed => {}
                    crate::gamemenu::MouseOutcome::None => self.game_advance_click(),
                }
            }
            (Phase::Game, Action::MouseRelease) => {
                let (x, y) = self.cursor.unwrap_or((-1.0, -1.0));
                let allowed = self.game_menu_allowed();
                let prohibit = self.vm_prohibit_save();
                if let crate::gamemenu::MouseOutcome::Command(i) = self
                    .game_menu
                    .mouse(x, y, false, allowed, prohibit)
                {
                    self.gamemenu_click(i);
                }
            }
            (Phase::Game, Action::Advance) => self.game_advance_click(),
            (Phase::Game, Action::RightClick) => {
                self.request_context_menu();
            }
            (Phase::Game, Action::MiddleRelease | Action::ToggleMessagePanel) => {
                self.toggle_message_panel();
            }
            (Phase::Game, Action::Up | Action::PageUp) => self.enter_backlog(),
            (Phase::Game, Action::MouseWheel { dy, .. }) if dy > 0.0 => {
                self.enter_backlog()
            }
            (Phase::Game, Action::MouseWheel { dy, .. }) if dy < 0.0 => {
                self.game_advance_click()
            }
            (Phase::Game, Action::MouseMove { x, y }) => {
                self.cursor = Some((x, y));
                let allowed = self.game_menu_allowed();
                let prohibit = self.vm_prohibit_save();
                if let crate::gamemenu::MouseOutcome::Command(i) = self
                    .game_menu
                    .mouse(x, y, false, allowed, prohibit)
                {
                    self.gamemenu_click(i);
                }
            }
            (_, Action::CtrlHeld(on)) => self.fast_forward = on,
            _ => {}
        }
    }
    fn key_pressed(&mut self, key: InputKey, repeat: bool) {
        if self.scene.borrow().movie.is_some() {
            if !repeat {
                self.movie_key_armed = true;
            }
            return;
        }
        let action = match key {
            InputKey::Enter => Action::Advance,
            InputKey::Space if matches!(self.phase, Phase::Title { .. }) => {
                Action::Advance
            }
            InputKey::Space => Action::ToggleMessagePanel,
            InputKey::Escape => Action::Cancel,
            InputKey::ArrowUp => Action::Up,
            InputKey::ArrowDown => Action::Down,
            InputKey::ArrowLeft => Action::Left,
            InputKey::ArrowRight => Action::Right,
            InputKey::PageUp => Action::PageUp,
            InputKey::PageDown => Action::PageDown,
            InputKey::Home => Action::HomeKey,
            InputKey::End => Action::EndKey,
            InputKey::Control => Action::CtrlHeld(true),
            InputKey::F5 => {
                let path = Self::save_dir().join("eden0001.sav");
                if let Err(err) = self.quick_save(&path) {
                    log::error!("快速存档失败 {}: {err:?}", path.display());
                }
                return;
            }
            InputKey::F9 => {
                let path = Self::save_dir().join("eden0001.sav");
                if path.exists() {
                    if let Err(err) = self.load(&path) {
                        log::error!("快速读档失败 {}: {err:?}", path.display());
                    }
                }
                return;
            }
            InputKey::Character(_) | InputKey::Other => return,
        };
        self.handle_action(action);
    }
    fn key_released(&mut self, key: InputKey) {
        if key == InputKey::Control {
            self.fast_forward = false;
        }
        if self.scene.borrow().movie.is_some() {
            let armed = std::mem::take(&mut self.movie_key_armed);
            if armed {
                self.finish_movie_from_release();
            }
        }
    }
    fn finish_movie_from_release(&mut self) -> bool {
        let skip_allowed = self
            .scene
            .borrow()
            .movie
            .as_ref()
            .map(|movie| match movie {
                crate::scene::MovieState::Pending(request)
                | crate::scene::MovieState::Playing(request) => request.skip_allowed,
            });
        let Some(skip_allowed) = skip_allowed else {
            return false;
        };
        if skip_allowed {
            self.finish_movie_wait();
        }
        true
    }
    fn pointer_left(&mut self) {
        self.cursor = None;
        self.menu_press = None;
        match &mut self.phase {
            Phase::Title { hover, .. } | Phase::Memories { hover } => *hover = None,
            Phase::Config(cfg, _) => cfg.hover = None,
            Phase::Game => self.game_menu.pointer_left(),
            _ => {}
        }
    }
    fn next_platform_request_id(&mut self) -> RequestId {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        id
    }
    fn request_context_menu(&mut self) {
        if self.active_context_menu.is_some() || !matches!(self.phase, Phase::Game)
            || self.backlog.is_some() || self.scene.borrow().movie.is_some()
            || self.vm.as_ref().is_some_and(|vm| vm.pragma_enabled("disable_control"))
        {
            return;
        }
        let request_id = self.next_platform_request_id();
        self.active_context_menu = Some(request_id);
        self.platform_requests
            .push_back(PlatformRequest::ShowContextMenu {
                request_id,
                state: ContextMenuState {
                    panel_hidden: self.message_panel_hidden,
                    panel_toggle_enabled: self.game_menu_allowed(),
                    auto_checked: self.adv_mode == 1,
                    skip_checked: self.adv_mode == 2,
                    save_load_enabled: !self.vm_prohibit_save(),
                    screen_mode: self.screen_mode,
                },
            });
    }
    fn context_menu_command(&mut self, command: ContextMenuCommand) {
        match command {
            ContextMenuCommand::ReturnToTitle => self.to_title(),
            ContextMenuCommand::ToggleMessagePanel => self.toggle_message_panel(),
            ContextMenuCommand::ToggleAuto => {
                self.adv_mode = if self.adv_mode == 1 { 0 } else { 1 };
                self.auto_ticks = 0;
            }
            ContextMenuCommand::ToggleSkip => {
                self.adv_mode = if self.adv_mode == 2 { 0 } else { 2 };
                self.auto_ticks = 0;
            }
            ContextMenuCommand::QuickSave => self.gamemenu_click(1),
            ContextMenuCommand::Save => self.gamemenu_click(4),
            ContextMenuCommand::Load => self.gamemenu_click(3),
            ContextMenuCommand::Config => self.gamemenu_click(2),
            ContextMenuCommand::ScreenBorderless => {
                self.context_screen_mode(ScreenMode::Borderless)
            }
            ContextMenuCommand::ScreenWindowed => {
                self.context_screen_mode(ScreenMode::Windowed)
            }
            ContextMenuCommand::Help => {
                self.platform_requests.push_back(PlatformRequest::OpenHelp)
            }
            ContextMenuCommand::Website => {
                self.platform_requests
                    .push_back(PlatformRequest::OpenUrl {
                        url: "http://www.minori.ph/".into(),
                    });
            }
            ContextMenuCommand::Exit => self.request_exit(),
        }
    }
    fn request_screen_mode(&mut self, mode: ScreenMode) {
        if self.screen_mode == mode {
            return;
        }
        self.pending_screen_mode = Some(mode);
        self.platform_requests
            .push_back(PlatformRequest::SetScreenMode {
                mode,
            });
    }
    fn context_screen_mode(&mut self, mode: ScreenMode) {
        if self.screen_mode.is_fullscreen() {
            self.request_screen_mode(ScreenMode::Windowed);
        } else {
            self.request_screen_mode(mode);
        }
    }
    pub fn request_exit(&mut self) {
        queue_exit(&mut self.quit, &mut self.pending_exit, &mut self.platform_requests);
    }
    pub fn notify_focus_lost(&mut self) {
        self.focused = false;
        self.menu_press = None;
        if self.pending_exit || self.quit {
            return;
        }
        if self.screen_mode.is_fullscreen() {
            self.request_screen_mode(ScreenMode::Windowed);
        }
    }
    pub fn notify_focus_gained(&mut self) {
        self.focused = true;
    }
    fn background_progress_allowed(&self) -> bool {
        background_progress_allowed(self.focused, &self.settings)
    }
    fn stop_voice_for_advance(&mut self) {
        stop_voice_on_advance(&self.settings, &self.audio);
    }
    fn save_window_geometry(&mut self, geometry: WindowGeometry) {
        let window_mode = if geometry.maximized {
            3
        } else {
            self.screen_mode.as_setting()
        };
        {
            let mut settings = self.settings.borrow_mut();
            settings.window_x = i64::from(geometry.x);
            settings.window_y = i64::from(geometry.y);
            settings.window_width = i64::from(geometry.width);
            settings.window_height = i64::from(geometry.height);
            settings.window_mode = window_mode;
        }
        persist_settings_to(self.storage.as_ref(), &self.settings.borrow());
    }
    fn toggle_message_panel(&mut self) {
        self.adv_mode = 0;
        self.auto_ticks = 0;
        self.message_panel_hidden = !self.message_panel_hidden;
    }
    fn enter_backlog(&mut self) {
        if !matches!(self.phase, Phase::Game)
            || self.vm.as_ref().is_some_and(|vm| vm.pragma_enabled("disable_control"))
        {
            return;
        }
        let entries = self
            .vm
            .as_ref()
            .map(|vm| vm.backlog().to_vec())
            .unwrap_or_default();
        if let Some(view) = BacklogView::new(entries) {
            self.adv_mode = 0;
            self.auto_ticks = 0;
            self.audio.borrow_mut().stop_voice();
            self.backlog = Some(view);
        }
    }
    fn exit_backlog(&mut self) {
        let captured = self.backlog.as_ref().is_some_and(|view| view.dragging_gauge);
        self.backlog = None;
        self.audio.borrow_mut().stop_voice();
        if captured {
            self.platform_requests
                .push_back(PlatformRequest::SetPointerCapture {
                    enabled: false,
                });
        }
    }
    fn handle_backlog_action(&mut self, input: Action) {
        match input {
            Action::MouseMove { x, y } => {
                self.cursor = Some((x, y));
                if let Some(view) = &mut self.backlog {
                    if view.dragging_gauge {
                        view.set_from_gauge_y(y);
                    }
                }
            }
            Action::MouseLeft => self.cursor = None,
            Action::Click { .. } => {
                let on_gauge = self
                    .cursor
                    .is_some_and(|(x, y)| in_rect(x, y, GAUGE_RECT));
                if on_gauge {
                    if let (Some((_, y)), Some(view)) = (
                        self.cursor,
                        &mut self.backlog,
                    ) {
                        view.dragging_gauge = true;
                        view.set_from_gauge_y(y);
                        self.platform_requests
                            .push_back(PlatformRequest::SetPointerCapture {
                                enabled: true,
                            });
                    }
                } else {
                    self.play_backlog_voice();
                }
            }
            Action::MouseRelease => {
                if let Some(view) = &mut self.backlog {
                    if view.dragging_gauge {
                        view.dragging_gauge = false;
                        self.platform_requests
                            .push_back(PlatformRequest::SetPointerCapture {
                                enabled: false,
                            });
                    }
                }
            }
            Action::MouseWheel { dy, .. } if dy > 0.0 => {
                if let Some(view) = &mut self.backlog {
                    view.older(dy.abs().round().max(1.0) as usize);
                }
            }
            Action::MouseWheel { dy, .. } if dy < 0.0 => {
                let stay = self
                    .backlog
                    .as_mut()
                    .is_some_and(|view| view.newer(dy.abs().round().max(1.0) as usize));
                if !stay {
                    self.exit_backlog();
                }
            }
            Action::Up | Action::PageUp => {
                if let Some(view) = &mut self.backlog {
                    view.older(1);
                }
            }
            Action::Down | Action::PageDown => {
                if !self.backlog.as_mut().is_some_and(|view| view.newer(1)) {
                    self.exit_backlog();
                }
            }
            Action::HomeKey => {
                if let Some(view) = &mut self.backlog {
                    view.oldest();
                }
            }
            Action::EndKey => {
                if let Some(view) = &mut self.backlog {
                    view.newest();
                }
            }
            Action::Advance => self.play_backlog_voice(),
            Action::Cancel | Action::RightClick => self.exit_backlog(),
            Action::CtrlHeld(on) => self.fast_forward = on,
            _ => {}
        }
    }
    fn play_backlog_voice(&mut self) {
        if self.settings.borrow().backlog_voice_playback == 0 {
            return;
        }
        let voice = self
            .backlog
            .as_ref()
            .map(|view| view.current().voice.clone())
            .unwrap_or_default();
        if voice.is_empty() {
            return;
        }
        let tail = voice.rsplit('\\').next().unwrap_or(&voice);
        let settings = self.settings.borrow();
        let (name, volume, pan) = parse_vol_pan(tail, settings.volume_voice);
        let muted = settings.mute_voice;
        drop(settings);
        self.audio.borrow_mut().stop_voice();
        if !muted {
            self.audio.borrow_mut().play_voice(&name, volume, pan);
        }
    }
    pub fn quick_save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.save_to(path, "クイックセーブ")
    }
    fn save_to(&mut self, path: &Path, comment: &str) -> anyhow::Result<()> {
        let Some(vm) = &self.vm else {
            anyhow::bail!("VM 未运行");
        };
        let file = eden_formats::SaveFile {
            header: eden_formats::SaveHeader {
                comment: comment.into(),
                route: [0; 4],
            },
            body: vm.save_data(),
        };
        let bytes = if self
            .native_vfs
            .as_ref()
            .is_some_and(|vfs| vfs.has_script_overrides())
        {
            eden_formats::write_port_sav(&file)
        } else {
            eden_formats::write_sav(&file)
        };
        #[cfg(target_arch = "wasm32")]
        {
            let save_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "存档路径没有有效文件名: {}", path.display()
                    )
                })?;
            self.platform_requests
                .push_back(PlatformRequest::PersistSave {
                    save_name: save_name.to_owned(),
                    bytes,
                });
            log::info!("Web 存档已进入异步持久化队列 → {save_name}");
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.write_persistent(path, &bytes)?;
            let vp = Viewport::new(
                self.renderer.config_width(),
                self.renderer.config_height(),
            );
            let (draw, mask_tr) = self.compose(&vp);
            let names: Vec<&str> = draw
                .sprites
                .iter()
                .chain(draw.sprites2.iter())
                .map(|s| s.image.as_str())
                .chain(draw.effects.iter().map(|effect| effect.image.as_str()))
                .chain(mask_tr.as_ref().map(|m| m.image.as_str()))
                .chain(mask_tr.as_ref().map(|m| m.mask.as_str()))
                .collect();
            self.renderer.prefetch(self.resources.as_ref(), &names);
            match self
                .renderer
                .render_thumbnail_png(
                    &draw,
                    mask_tr.as_ref(),
                    &mut self.atlas,
                    self.resources.as_ref(),
                )
            {
                Ok(png) => {
                    let thumb = path.with_extension("png");
                    self.write_persistent(&thumb, &png)?;
                    self.resources.invalidate_image(&thumb.to_string_lossy());
                    if let Some(name) = thumb.file_name().and_then(|name| name.to_str())
                    {
                        self.resources.invalidate_image(&format!("~save/{name}"));
                    }
                    log::info!("缩略图 → {}", thumb.display());
                }
                Err(e) => log::warn!("缩略图渲染失败: {e:?}"),
            }
            log::info!("存档 → {}", path.display());
            Ok(())
        }
    }
    pub fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        let bytes = self.read_persistent(path)?;
        let file = eden_formats::read_save_file(&bytes)?;
        let script = file
            .body
            .variables
            .iter()
            .find(|(k, _)| k == "script_Filename")
            .map(|(_, v)| v.clone())
            .ok_or_else(|| anyhow::anyhow!("存档缺 script_Filename"))?;
        self.restart_vm(&script);
        let (snapshot, pc) = {
            let Some(vm) = &mut self.vm else { unreachable!() };
            vm.restore_save(&file.body);
            *self.scene.borrow_mut() = Scene::rebuild_from_system(&vm.system_snapshot());
            let _ = vm.step();
            (vm.system_snapshot(), vm.pc())
        };
        self.audio.borrow_mut().stop_voice();
        self.restore_saved_loop_audio(&snapshot);
        self.phase = Phase::Game;
        self.backlog = None;
        self.message_panel_hidden = false;
        self.active_context_menu = None;
        self.pending_save_comment = None;
        self.game_menu = Default::default();
        self.adv_mode = 0;
        self.auto_ticks = 0;
        log::info!("读档 ← {}（{}:{}）", path.display(), script, pc);
        Ok(())
    }
    pub fn render(&mut self) {
        self.frame_no += 1;
        if std::env::var_os("EDEN_TRACE").is_some() && self.frame_no % 300 == 0 {
            log::info!(
                "[trace] frame {} +{:.2}s phase={}", self.frame_no, self.start.elapsed()
                .as_secs_f32(), self.phase_variant()
            );
        }
        let vp = Viewport::new(
            self.renderer.config_width(),
            self.renderer.config_height(),
        );
        let (mut draw, mut mask_tr) = self.compose(&vp);
        let shaking = self.scene.borrow().shake.is_some();
        if !shaking {
            self.shake_snapshot = None;
        } else if self.shake_snapshot.is_none() {
            self.shake_snapshot = Some(ShakeSnapshot {
                draw: draw.clone(),
                mask: mask_tr.clone(),
            });
        }
        if let Some(snapshot) = &self.shake_snapshot {
            draw = snapshot.draw.clone();
            mask_tr = snapshot.mask.clone();
        }
        let (shx, shy, zoomed, zoom_amt) = self.scene.borrow().shake_visual();
        if zoomed {
            scale_frame_draw(&mut draw, mask_tr.as_mut(), zoom_amt);
        }
        if shx != 0.0 || shy != 0.0 {
            translate_frame_draw(&mut draw, mask_tr.as_mut(), shx, shy);
        }
        let names: Vec<&str> = draw
            .sprites
            .iter()
            .chain(draw.sprites2.iter())
            .map(|s| s.image.as_str())
            .chain(draw.effects.iter().map(|effect| effect.image.as_str()))
            .chain(mask_tr.as_ref().map(|m| m.image.as_str()))
            .chain(mask_tr.as_ref().map(|m| m.mask.as_str()))
            .collect();
        self.renderer.prefetch(self.resources.as_ref(), &names);
        let now = self.start.elapsed();
        let shot = self
            .shots
            .iter()
            .position(|(t, _)| *t <= now)
            .map(|i| self.shots.remove(i));
        if let Some((_, path)) = &shot {
            if let Err(e) = self
                .renderer
                .render_offscreen(
                    &vp,
                    &draw,
                    mask_tr.as_ref(),
                    &mut self.atlas,
                    self.resources.as_ref(),
                    path,
                )
            {
                log::error!("离屏截图失败: {e:?}");
            }
        }
        if let Some(every) = self.auto_advance_every {
            if now >= self.next_auto {
                self.next_auto = now + every;
                if matches!(
                    self.phase, Phase::Title { .. } | Phase::SaveLoad(_) |
                    Phase::Memories { .. } | Phase::CgGallery(_) | Phase::MusicMenu(_)
                ) {
                    self.input(InputEvent::Key {
                        key: InputKey::Enter,
                        state: ButtonState::Pressed,
                        repeat: false,
                    });
                } else if self.scene.borrow().movie.is_some() {
                    self.game_advance_click();
                } else {
                    let typing = {
                        let mut s = self.scene.borrow_mut();
                        match &mut s.message {
                            Some(m) if !m.done() => {
                                m.reveal_all();
                                true
                            }
                            _ => false,
                        }
                    };
                    if !typing {
                        if let Some(vm) = &mut self.vm {
                            vm.advance_input();
                        }
                    }
                }
            }
        }
        loop {
            let next_hover = self.hovers_at.first().map(|(t, _, _)| *t);
            let next_click = self.clicks_at.first().copied();
            match (next_hover, next_click) {
                (Some(h), Some(c)) if h <= c && h <= now => {
                    let (_, x, y) = self.hovers_at.remove(0);
                    self.input(InputEvent::PointerMoved { x, y });
                }
                (_, Some(c)) if c <= now => {
                    self.clicks_at.remove(0);
                    self.input(InputEvent::PointerButton {
                        button: PointerButton::Left,
                        state: ButtonState::Pressed,
                        click_count: 1,
                    });
                    self.input(InputEvent::PointerButton {
                        button: PointerButton::Left,
                        state: ButtonState::Released,
                        click_count: 1,
                    });
                }
                _ => break,
            }
        }
        if let Some((t, path)) = &self.save_at {
            if now >= *t {
                let path = path.clone();
                self.save_at = None;
                if let Err(e) = self.quick_save(&path) {
                    log::error!("EDEN_SAVE 快存失败: {e:?}");
                }
            }
        }
        if let Some(until) = self.exit_after {
            if now >= until {
                self.quit = true;
            }
        }
        if let Some(path) = std::env::var_os("EDEN_DUMP_ATLAS") {
            let (aw, ah) = self.atlas.atlas_dims();
            let data = self.atlas.atlas_data().to_vec();
            let path = std::path::PathBuf::from(path);
            let _ = std::fs::write(&path, encode_gray_png(&data, aw, ah));
            std::env::remove_var("EDEN_DUMP_ATLAS");
            log::info!("图集导出 → {}", path.display());
        }
        if let Err(e) = self
            .renderer
            .render(
                &vp,
                &draw,
                mask_tr.as_ref(),
                &mut self.atlas,
                self.resources.as_ref(),
            )
        {
            log::error!("渲染失败: {e:?}");
        }
    }
    fn compose_scene_layer(
        &mut self,
        vp: &Viewport,
        sprites: &mut Vec<SpriteDraw>,
        glyphs: &mut Vec<GlyphQuad>,
        panel_sprites2: &mut Vec<SpriteDraw>,
        panel_glyphs2: &mut Vec<GlyphQuad>,
        mask_tr: &mut Option<MaskTransitionDraw>,
        effects: &mut Vec<EffectSpriteDraw>,
    ) {
        let scene = self
            .backlog
            .as_ref()
            .map(|view| view.scene().clone())
            .unwrap_or_else(|| self.scene.borrow().clone());
        let backlog_active = self.backlog.is_some();
        let alpha = scene.transition_alpha();
        if let Some(prev) = &scene.prev_bg {
            sprites.push(self.bg_sprite(prev));
            if let Some(cur) = &scene.bg {
                if let Some(tr) = &scene.active_tr {
                    if tr.spec.kind == 1 && tr.spec.mask.is_some() {
                        *mask_tr = Some(MaskTransitionDraw {
                            image: cur.name.clone(),
                            mask: tr.spec.mask.clone().unwrap(),
                            rect: [0.0, 0.0, 1024.0, 640.0],
                            progress: alpha,
                        });
                    } else {
                        sprites.push(self.bg_sprite_alpha(cur, alpha));
                    }
                }
            }
        } else if let Some(cur) = &scene.bg {
            sprites.push(self.bg_sprite(cur));
            if scene.active_tr.is_none() {
                self.push_cg_patches(&scene, sprites);
            }
        }
        if scene.movie.is_some() {
            sprites.push(sdraw("BLACK.png"));
        } else {
            if let Some(panel_tr) = &scene.panel_tr {
                if !self.message_panel_hidden || backlog_active {
                    let alpha = scene.panel_alpha();
                    let awaiting = !backlog_active
                        && self
                            .vm
                            .as_ref()
                            .is_some_and(|vm| {
                                matches!(vm.state(), RunState::AwaitInput)
                            });
                    push_panel_layer(
                        &mut self.atlas,
                        self.resources.as_ref(),
                        self.frame_no,
                        vp,
                        sprites,
                        glyphs,
                        scene.message.as_ref(),
                        panel_tr.from,
                        1.0 - alpha,
                        awaiting,
                    );
                    push_panel_layer(
                        &mut self.atlas,
                        self.resources.as_ref(),
                        self.frame_no,
                        vp,
                        panel_sprites2,
                        panel_glyphs2,
                        scene.message.as_ref(),
                        panel_tr.to,
                        alpha,
                        awaiting,
                    );
                }
            } else {
                let (
                    panel_img,
                    panel_rect,
                    msg_box,
                    name_box,
                ): (&str, [f32; 4], &TextBox, Option<&TextBox>) = match scene.panel_mode
                {
                    1 => {
                        (
                            panel_asset(1),
                            panel_rect_for_mode(1),
                            &MSG_BOX_PANEL1,
                            Some(&NAME_BOX_PANEL1),
                        )
                    }
                    2 => {
                        (
                            panel_asset(2),
                            panel_rect_for_mode(2),
                            &MSG_BOX_FULL,
                            Some(&NAME_BOX_FULL),
                        )
                    }
                    3 => {
                        (
                            panel_asset(3),
                            panel_rect_for_mode(3),
                            &MSG_BOX_BAR,
                            Some(&NAME_BOX_BAR),
                        )
                    }
                    5 => {
                        (
                            panel_asset(5),
                            panel_rect_for_mode(5),
                            &MSG_BOX_BAR,
                            Some(&NAME_BOX_BAR),
                        )
                    }
                    4 => (panel_asset(4), panel_rect_for_mode(4), &MSG_BOX_FULL2, None),
                    6 => {
                        (
                            panel_asset(6),
                            panel_rect_for_mode(6),
                            &MSG_BOX_BAR,
                            Some(&NAME_BOX_BAR),
                        )
                    }
                    _ => ("", [0.0; 4], &MSG_BOX_PANEL1, None),
                };
                if panel_text_visible(scene.panel_mode) && !panel_img.is_empty()
                    && (!self.message_panel_hidden || backlog_active)
                {
                    sprites
                        .push(SpriteDraw {
                            image: panel_img.into(),
                            rect: panel_rect,
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: [1.0; 4],
                        });
                }
                if panel_text_visible(scene.panel_mode)
                    && (!self.message_panel_hidden || backlog_active)
                {
                    if let Some(msg) = &scene.message {
                        let name = msg
                            .speaker
                            .as_deref()
                            .and_then(|s| speaker_display_for_panel(
                                s,
                                scene.panel_mode,
                            ));
                        if let (Some(box_), Some(shown)) = (name_box, name.as_deref()) {
                            glyphs
                                .extend(
                                    layout_ex(
                                            &mut self.atlas,
                                            vp,
                                            shown,
                                            box_,
                                            22.0,
                                            10.0,
                                            [1.0, 1.0, 1.0, 1.0],
                                            false,
                                            0,
                                            None,
                                        )
                                        .quads,
                                );
                        }
                        let first_line = usize::from(
                            name.is_some() && scene.panel_mode == 1,
                        );
                        let body = layout_ex(
                            &mut self.atlas,
                            vp,
                            &msg.body,
                            msg_box,
                            22.0,
                            10.0,
                            [1.0, 1.0, 1.0, 1.0],
                            true,
                            first_line,
                            Some(msg.revealed),
                        );
                        glyphs.extend(body.quads);
                        let awaiting = !backlog_active
                            && self
                                .vm
                                .as_ref()
                                .is_some_and(|vm| {
                                    matches!(vm.state(), RunState::AwaitInput)
                                });
                        if msg.done() && awaiting {
                            if let Some((pen_x, line_top)) = body.cursor {
                                const CURSOR_FRAME: f32 = 22.0;
                                let strip = if (scene.panel_mode == 2
                                    || scene.panel_mode == 4) && !msg.is_read
                                {
                                    "pauseAnimeNextPage.png"
                                } else {
                                    "pauseAnime.png"
                                };
                                let strip_w = self
                                    .resources
                                    .request_image(strip)
                                    .ready()
                                    .map(|image| image.logical_w)
                                    .unwrap_or(264.0);
                                let frames = (strip_w / CURSOR_FRAME).max(1.0) as u64;
                                let x = ((pen_x + 5.0) as i32 & !1) as f32;
                                let y = line_top + 31.0 - CURSOR_FRAME + 2.0;
                                let f = ((self.frame_no / 10) % frames) as f32;
                                sprites
                                    .push(SpriteDraw {
                                        image: strip.into(),
                                        rect: [x, y, CURSOR_FRAME, CURSOR_FRAME],
                                        uv: [
                                            f * CURSOR_FRAME / strip_w,
                                            0.0,
                                            CURSOR_FRAME / strip_w,
                                            1.0,
                                        ],
                                        color: [1.0; 4],
                                    });
                            }
                        }
                    }
                }
            }
        }
        if let Some(firefly) = &scene.firefly {
            for particle in &firefly.particles {
                let alpha = particle.alpha as u32 * firefly.alpha as u32 / 255;
                if alpha == 0 {
                    continue;
                }
                effects
                    .push(EffectSpriteDraw {
                        image: crate::scene::FIREFLY_IMAGES[particle.texture]
                            .to_string(),
                        x: particle.x,
                        y: particle.y,
                        alpha: alpha as u16,
                    });
            }
        }
    }
    fn compose(&mut self, vp: &Viewport) -> (FrameDraw, Option<MaskTransitionDraw>) {
        let mut sprites: Vec<SpriteDraw> = Vec::new();
        let mut glyphs: Vec<GlyphQuad> = Vec::new();
        let mut sprites2: Vec<SpriteDraw> = Vec::new();
        let mut glyphs2: Vec<GlyphQuad> = Vec::new();
        let mut effects: Vec<EffectSpriteDraw> = Vec::new();
        let mut mask_tr: Option<MaskTransitionDraw> = None;
        let under = match &self.phase {
            Phase::SaveLoad(menu) => Some(menu.from_game),
            Phase::Config(_, prev) => Some(matches!(** prev, Phase::Game)),
            _ => None,
        };
        match under {
            Some(true) => {
                self.compose_scene_layer(
                    vp,
                    &mut sprites,
                    &mut glyphs,
                    &mut sprites2,
                    &mut glyphs2,
                    &mut mask_tr,
                    &mut effects,
                )
            }
            Some(false) => {
                let idx = title_bg_index(
                    global_flag(&self.progress, "eden_D001"),
                    global_flag(&self.progress, "eden_B006"),
                );
                sprites
                    .push(SpriteDraw {
                        image: format!("topmenu{idx}.png"),
                        rect: [0.0, 0.0, 1024.0, 640.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: [1.0; 4],
                    });
            }
            None => {}
        }
        match &mut self.phase {
            Phase::Title { selection: _, hover } => {
                let idx = title_bg_index(
                    global_flag(&self.progress, "eden_D001"),
                    global_flag(&self.progress, "eden_B006"),
                );
                let base_name = format!("topmenu{idx}.png");
                let over_name = format!("topmenu{idx}over.png");
                let alpha = (self.title_fade as f32 / 255.0).clamp(0.0, 1.0);
                let item = *hover;
                let names: Vec<&str> = if item.is_some() {
                    vec![base_name.as_str(), over_name.as_str()]
                } else {
                    vec![base_name.as_str()]
                };
                self.renderer.prefetch(self.resources.as_ref(), &names);
                sprites
                    .push(SpriteDraw {
                        image: base_name,
                        rect: [0.0, 0.0, 1024.0, 640.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: [alpha, alpha, alpha, alpha],
                    });
                if let Some(i) = item {
                    let r = TITLE_HIT[i];
                    sprites
                        .push(SpriteDraw {
                            image: over_name,
                            rect: [r[0], r[1], r[2] - r[0], r[3] - r[1]],
                            uv: [
                                r[0] / 1024.0,
                                r[1] / 640.0,
                                (r[2] - r[0]) / 1024.0,
                                (r[3] - r[1]) / 640.0,
                            ],
                            color: [alpha, alpha, alpha, alpha],
                        });
                }
            }
            Phase::SaveLoad(menu) => {
                let a = menu.alpha();
                let ac = [a, a, a, a];
                let base = if menu.mode == 0 {
                    "saveBase0.png"
                } else {
                    "loadBase0.png"
                };
                sprites2
                    .push(SpriteDraw {
                        image: base.into(),
                        rect: [0.0, 0.0, 1024.0, 640.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: ac,
                    });
                for (i, s) in menu.slots.iter().enumerate() {
                    let r = SLOT_RECTS[i];
                    let img = if s.exists {
                        s.thumb.to_string_lossy().into_owned()
                    } else {
                        "notsaved.png".into()
                    };
                    sprites2
                        .push(SpriteDraw {
                            image: img,
                            rect: [r[0] + 10.0, r[1] + 11.0, 128.0, 80.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                    if !s.exists {
                        continue;
                    }
                    let abs = (menu.page * 10 + i) as i32;
                    let red = abs == menu.oldest || abs == menu.newest;
                    let color = if red {
                        [1.0, 0.0, 0.0, a]
                    } else {
                        [0.0, 0.0, 0.0, a]
                    };
                    let mut text = s.timestamp.clone();
                    if !s.comment.is_empty() {
                        text.push('\n');
                        text.push_str(&s.comment);
                    }
                    let quads = layout_ex(
                            &mut self.atlas,
                            vp,
                            &text,
                            &TextBox {
                                ox: 0.0,
                                oy: 0.0,
                                rect: [r[0] + 152.0, r[1], r[2], r[3]],
                            },
                            20.0,
                            10.0,
                            color,
                            false,
                            0,
                            None,
                        )
                        .quads;
                    glyphs2.extend(quads);
                }
                if menu.selection >= 0 {
                    let r = SLOT_RECTS[menu.selection as usize];
                    sprites2
                        .push(SpriteDraw {
                            image: "saveloadSelect.png".into(),
                            rect: [r[0], r[1], 344.0, 98.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                }
                if menu.page <= 9 {
                    let tab = format!("slpage0{}.png", menu.page);
                    let (tw, u0) = if menu.mode == 0 {
                        (78.0, 42.0 / 120.0)
                    } else {
                        (120.0, 0.0)
                    };
                    sprites2
                        .push(SpriteDraw {
                            image: tab,
                            rect: [crate::saveload::tab_x(menu.mode), 580.0, tw, 40.0],
                            uv: [u0, 0.0, tw / 120.0, 1.0],
                            color: ac,
                        });
                }
                let left_on = menu.page >= 1 && (menu.mode != 0 || menu.page >= 2);
                let right_on = menu.page < 9;
                let seg = |x0: f32, x1: f32| SpriteDraw {
                    image: "slmenu0.png".into(),
                    rect: [512.0 + x0, 579.0, x1 - x0, 61.0],
                    uv: [x0 / 512.0, 0.0, (x1 - x0) / 512.0, 1.0],
                    color: ac,
                };
                if left_on {
                    sprites2.push(seg(0.0, 128.0));
                }
                sprites2.push(seg(128.0, 176.0));
                if right_on {
                    sprites2.push(seg(176.0, 280.0));
                }
                sprites2.push(seg(280.0, 512.0));
            }
            Phase::Memories { hover } => {
                sprites
                    .push(SpriteDraw {
                        image: "memories.png".into(),
                        rect: [0.0, 0.0, 1024.0, 640.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: [1.0; 4],
                    });
                for &i in hover.iter() {
                    let r = MEMORIES_HIT[i];
                    sprites
                        .push(SpriteDraw {
                            image: "memoriesOver.png".into(),
                            rect: [r[0], r[1], r[2] - r[0], r[3] - r[1]],
                            uv: [
                                r[0] / 1024.0,
                                r[1] / 640.0,
                                (r[2] - r[0]) / 1024.0,
                                (r[3] - r[1]) / 640.0,
                            ],
                            color: [1.0; 4],
                        });
                }
            }
            Phase::CgGallery(g) => {
                if let Some(v) = &g.viewer {
                    let alpha = v.alpha();
                    if let Some(name) = v.current() {
                        let (tw, th) = self
                            .resources
                            .request_image(name)
                            .ready()
                            .map(|i| i.logical_size())
                            .unwrap_or((1024.0, 640.0));
                        let x = (v.win_x as f32).clamp(0.0, (tw - 1024.0).max(0.0));
                        let y = (v.win_y as f32).clamp(0.0, (th - 640.0).max(0.0));
                        sprites
                            .push(SpriteDraw {
                                image: name.to_string(),
                                rect: [0.0, 0.0, 1024.0, 640.0],
                                uv: [
                                    x / tw,
                                    y / th,
                                    1024.0_f32.min(tw) / tw,
                                    640.0_f32.min(th) / th,
                                ],
                                color: [alpha, alpha, alpha, alpha],
                            });
                    }
                } else {
                    let a = g.alpha();
                    let ac = [a, a, a, a];
                    let base = format!("cgmode{}.png", g.mode);
                    if self.resources.request_image(&base).ready().is_some() {
                        sprites
                            .push(SpriteDraw {
                                image: base,
                                rect: [0.0, 0.0, 1024.0, 640.0],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    } else {
                        sprites
                            .push(SpriteDraw {
                                image: "BLACK.png".into(),
                                rect: [0.0, 0.0, 1024.0, 640.0],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: [a, a, a, a],
                            });
                    }
                    let sheet = format!("cgmode{}box.png", g.mode);
                    let has_sheet = self
                        .resources
                        .request_image(&sheet)
                        .ready()
                        .is_some();
                    let seen = self.cg_seen.borrow();
                    for cell in 0..25usize {
                        let Some(entry) = g.cell_entry(cell) else {
                            break;
                        };
                        let (x, y) = (
                            crate::cgmode::GRID_XS[cell % 5],
                            crate::cgmode::GRID_YS[cell / 5],
                        );
                        if has_sheet {
                            sprites
                                .push(SpriteDraw {
                                    image: sheet.clone(),
                                    rect: [x, y, crate::cgmode::CELL_W, crate::cgmode::CELL_H],
                                    uv: [
                                        x / 1024.0,
                                        y / 640.0,
                                        crate::cgmode::CELL_W / 1024.0,
                                        crate::cgmode::CELL_H / 640.0,
                                    ],
                                    color: ac,
                                });
                        }
                        let name = crate::cgmode::CgGallery::display_name(entry);
                        if seen.contains(name) {
                            let thumb = format!("cgthumb\\{name}");
                            if self.resources.request_image(&thumb).ready().is_some() {
                                sprites
                                    .push(SpriteDraw {
                                        image: thumb,
                                        rect: [
                                            x,
                                            y,
                                            crate::cgmode::THUMB_W,
                                            crate::cgmode::THUMB_H,
                                        ],
                                        uv: [0.0, 0.0, 1.0, 1.0],
                                        color: ac,
                                    });
                            } else {
                                sprites
                                    .push(SpriteDraw {
                                        image: "BLACK.png".into(),
                                        rect: [
                                            x,
                                            y,
                                            crate::cgmode::THUMB_W,
                                            crate::cgmode::THUMB_H,
                                        ],
                                        uv: [0.0, 0.0, 1.0, 1.0],
                                        color: [0.0, 1.0, 0.0, 100.0 / 255.0 * a],
                                    });
                            }
                        } else {
                            sprites
                                .push(SpriteDraw {
                                    image: "BLACK.png".into(),
                                    rect: [
                                        x,
                                        y,
                                        crate::cgmode::THUMB_W,
                                        crate::cgmode::THUMB_H,
                                    ],
                                    uv: [0.0, 0.0, 1.0, 1.0],
                                    color: [
                                        0x20 as f32 / 255.0,
                                        0x20 as f32 / 255.0,
                                        0x20 as f32 / 255.0,
                                        a,
                                    ],
                                });
                        }
                    }
                    let page_img = format!("cgpage{}{:02}.png", g.mode, g.page + 1);
                    if let Some(img) = self.resources.request_image(&page_img).ready() {
                        let (pw, ph) = img.logical_size();
                        sprites
                            .push(SpriteDraw {
                                image: page_img,
                                rect: [491.0, 575.0, pw, ph],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    }
                    let menu = format!("cgmode{}menu.png", g.mode);
                    if let Some(img) = self.resources.request_image(&menu).ready() {
                        let (mw, mh) = img.logical_size();
                        sprites
                            .push(SpriteDraw {
                                image: menu,
                                rect: [1024.0 - mw, 640.0 - mh, mw, mh],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    }
                }
            }
            Phase::MusicMenu(m) => {
                let a = m.alpha();
                let ac = [a, a, a, a];
                let base = m.base();
                if self.resources.request_image(&base).ready().is_some() {
                    sprites
                        .push(SpriteDraw {
                            image: base,
                            rect: [0.0, 0.0, 1024.0, 640.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                }
                if (0..16).contains(&m.highlight) {
                    sprites
                        .push(SpriteDraw {
                            image: "musicNote.png".into(),
                            rect: [99.0, 40.0 + 32.0 * m.highlight as f32, 10.0, 15.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                }
                if m.playing {
                    let secs = m.elapsed;
                    draw_digits(
                        &mut sprites,
                        "musicLargeFont.png",
                        32.0,
                        48.0,
                        (secs / 60) as u32,
                        crate::music::TIME_MIN_AT,
                        ac,
                    );
                    draw_digits(
                        &mut sprites,
                        "musicLargeFont.png",
                        32.0,
                        48.0,
                        secs % 60,
                        crate::music::TIME_SEC_AT,
                        ac,
                    );
                    let track = (m.playing_abs.max(0) as usize)
                        % crate::music::MODE_TRACKS + 1;
                    draw_digits(
                        &mut sprites,
                        "musicSmallFont.png",
                        16.0,
                        16.0,
                        track as u32,
                        crate::music::TRACK_NO_AT,
                        ac,
                    );
                    for (i, &lvl) in m.spectrum.iter().enumerate() {
                        let h = (lvl * 75.0).round();
                        if h < 1.0 {
                            continue;
                        }
                        let x = crate::music::METER_RECT[0] + 8.0 * i as f32;
                        let y1 = crate::music::METER_RECT[3];
                        sprites
                            .push(SpriteDraw {
                                image: "BLACK.png".into(),
                                rect: [x, y1 - h, 8.0, h],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: [
                                    0x64 as f32 / 255.0,
                                    0x4B as f32 / 255.0,
                                    0x32 as f32 / 255.0,
                                    a,
                                ],
                            });
                        sprites
                            .push(SpriteDraw {
                                image: "BLACK.png".into(),
                                rect: [x + 1.0, y1 - h - 1.0, 6.0, 1.0],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: [1.0, 1.0, 1.0, a],
                            });
                    }
                }
            }
            Phase::Config(cfg, _) => {
                let a = cfg.alpha();
                let ac = [a, a, a, a];
                sprites2
                    .push(SpriteDraw {
                        image: "configBase.png".into(),
                        rect: [0.0, 0.0, 1024.0, 640.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: ac,
                    });
                if let Some((x, y)) = match cfg.hover {
                    Some(100) => Some((679.0, 541.0)),
                    Some(101) => Some((803.0, 541.0)),
                    _ => None,
                } {
                    let (cw, ch) = self
                        .resources
                        .request_image("circle.png")
                        .ready()
                        .map(|i| i.logical_size())
                        .unwrap_or((74.0, 58.0));
                    sprites2
                        .push(SpriteDraw {
                            image: "circle.png".into(),
                            rect: [x, y, cw, ch],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                }
                let s = self.settings.borrow();
                let vals = [
                    s.message_speed_tbr,
                    s.message_speed_read,
                    s.message_speed_autoplay,
                    s.volume_bgm,
                    s.volume_voice,
                    s.volume_se,
                ];
                let (kw, kh) = self
                    .resources
                    .request_image("knob.png")
                    .ready()
                    .map(|i| i.logical_size())
                    .unwrap_or((15.0, 25.0));
                for (i, &(_cmd, _hit_x0, base_x, y)) in crate::config::SLIDERS
                    .iter()
                    .enumerate()
                {
                    let v = vals[i].clamp(0, 100);
                    let x = base_x + (200 * v / 100) as f32;
                    sprites2
                        .push(SpriteDraw {
                            image: "knob.png".into(),
                            rect: [x, y, kw, kh],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: ac,
                        });
                }
                let (cw, ch) = self
                    .resources
                    .request_image("checkmark.png")
                    .ready()
                    .map(|i| i.logical_size())
                    .unwrap_or((21.0, 32.0));
                for (i, &(x, y)) in crate::config::MUTE_CHECKS.iter().enumerate() {
                    if [s.mute_bgm, s.mute_voice, s.mute_se][i] {
                        sprites2
                            .push(SpriteDraw {
                                image: "checkmark.png".into(),
                                rect: [x, y, cw, ch],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    }
                }
                for (i, &(x, y)) in crate::config::VOICE_CHECKS.iter().enumerate() {
                    if s.voice_chars[i] {
                        sprites2
                            .push(SpriteDraw {
                                image: "checkmark.png".into(),
                                rect: [x, y, cw, ch],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    }
                }
                for &(cmd, x, y) in crate::config::CHECKS.iter() {
                    if crate::config::check_state(&s, cmd) {
                        sprites2
                            .push(SpriteDraw {
                                image: "checkmark.png".into(),
                                rect: [x, y, cw, ch],
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: ac,
                            });
                    }
                }
                sprites2
                    .push(SpriteDraw {
                        image: "checkmark.png".into(),
                        rect: [
                            415.0,
                            if cfg.screen_mode == 0 { 112.0 } else { 68.0 },
                            cw,
                            ch,
                        ],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        color: ac,
                    });
                drop(s);
                let face_box = TextBox {
                    ox: 0.0,
                    oy: 0.0,
                    rect: [129.0, 374.0, 361.0, 422.0],
                };
                let sample_box = TextBox {
                    ox: 0.0,
                    oy: 0.0,
                    rect: [145.0, 418.0, 323.0, 500.0],
                };
                glyphs2
                    .extend(
                        layout_ex(
                                &mut self.atlas,
                                vp,
                                &cfg.font_name,
                                &face_box,
                                20.0,
                                10.0,
                                [1.0, 1.0, 1.0, a],
                                false,
                                0,
                                None,
                            )
                            .quads,
                    );
                glyphs2
                    .extend(
                        layout_ex(
                                &mut self.atlas,
                                vp,
                                "地球で最後の\\n　　　　恋物語",
                                &sample_box,
                                22.0,
                                10.0,
                                [1.0, 1.0, 1.0, a],
                                false,
                                0,
                                None,
                            )
                            .quads,
                    );
            }
            Phase::Game => {
                self.compose_scene_layer(
                    vp,
                    &mut sprites,
                    &mut glyphs,
                    &mut sprites2,
                    &mut glyphs2,
                    &mut mask_tr,
                    &mut effects,
                );
                if self.backlog.is_none() && self.scene.borrow().movie.is_none() {
                    let play_mode = self.settings.borrow().play_mode;
                    if let Some((rect, a)) = self.game_menu.back_draw() {
                        let base = if play_mode != 0 {
                            "gameMenuBackAuto.png"
                        } else {
                            "gameMenuBack.png"
                        };
                        sprites
                            .push(SpriteDraw {
                                image: base.into(),
                                rect,
                                uv: [0.0, 0.0, 1.0, 1.0],
                                color: [a, a, a, a],
                            });
                    }
                    for i in 0..crate::gamemenu::BUTTONS.len() {
                        if let Some((rect, a)) = self.game_menu.button_draw(i) {
                            let name = if i == 0 && play_mode != 0 {
                                "gameMenuAuto.png"
                            } else {
                                crate::gamemenu::BTN_NAMES[i]
                            };
                            sprites
                                .push(SpriteDraw {
                                    image: name.into(),
                                    rect,
                                    uv: [0.0, 0.0, 1.0, 1.0],
                                    color: [a, a, a, a],
                                });
                        }
                    }
                }
                if let Some(view) = &self.backlog {
                    sprites
                        .push(SpriteDraw {
                            image: "backlogGauge.png".into(),
                            rect: GAUGE_RECT,
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: [1.0; 4],
                        });
                    sprites
                        .push(SpriteDraw {
                            image: "ball.png".into(),
                            rect: [1008.0, view.ball_y(), 14.0, 14.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: [1.0; 4],
                        });
                }
                if let Some((name, _)) = &self.notify {
                    sprites
                        .push(SpriteDraw {
                            image: name.clone(),
                            rect: [0.0, 608.0, 20.0, 32.0],
                            uv: [0.0, 0.0, 1.0, 1.0],
                            color: [1.0; 4],
                        });
                }
            }
        }
        (
            FrameDraw {
                sprites,
                glyphs,
                sprites2,
                glyphs2,
                effects,
                movie: self.movie.borrow().current_frame(),
            },
            mask_tr,
        )
    }
}
impl Engine {
    fn phase_variant(&self) -> &str {
        match self.phase {
            Phase::Title { .. } => "Title",
            Phase::Game => "Game",
            Phase::SaveLoad(_) => "SaveLoad",
            Phase::Config(..) => "Config",
            Phase::Memories { .. } => "Memories",
            Phase::CgGallery(_) => "CgGallery",
            Phase::MusicMenu(_) => "MusicMenu",
        }
    }
    fn bg_sprite(&self, bg: &crate::scene::BgSlot) -> SpriteDraw {
        self.bg_sprite_alpha(bg, 1.0)
    }
    fn push_cg_patches(&self, scene: &Scene, sprites: &mut Vec<SpriteDraw>) {
        let (Some(bg), Some(anim), Some(ani)) = (&scene.bg, &scene.cg_anim, &self.cg_ani)
        else {
            return;
        };
        let (tw, th) = self
            .resources
            .request_image(&bg.name)
            .ready()
            .map(|i| i.logical_size())
            .unwrap_or((1024.0, 640.0));
        let win_x = (bg.win_x as f32).clamp(0.0, (tw - 1024.0).max(0.0));
        let win_y = (bg.win_y as f32).clamp(0.0, (th - 640.0).max(0.0));
        let push_patch = |sprites: &mut Vec<SpriteDraw>, label: &str| {
            let Some(idx) = ani.frames.iter().position(|f| f.label == label) else {
                return;
            };
            let Some((key, frame_x, frame_y, w, h)) = self
                .native_vfs
                .as_ref()
                .and_then(|vfs| vfs.ani_frame_image(&anim.png, ani, idx)) else {
                return;
            };
            let x = frame_x - win_x;
            let y = frame_y - win_y;
            if x + w <= 0.0 || x >= 1024.0 || y + h <= 0.0 || y >= 640.0 {
                return;
            }
            sprites
                .push(SpriteDraw {
                    image: key,
                    rect: [x, y, w, h],
                    uv: [0.0, 0.0, 1.0, 1.0],
                    color: [1.0; 4],
                });
        };
        for c in &anim.chars {
            let eye = crate::scene::EYE_WORDS[c.eye.state.clamp(0, 2) as usize];
            push_patch(sprites, &format!("{} {eye}", c.name));
            if c.mouth.state != 0 {
                let m = crate::scene::MOUTH_WORDS[c.mouth.state as usize];
                push_patch(sprites, &format!("{} {m}", c.name));
            }
        }
    }
    fn bg_sprite_alpha(&self, bg: &crate::scene::BgSlot, alpha: f32) -> SpriteDraw {
        let (tw, th) = self
            .resources
            .request_image(&bg.name)
            .ready()
            .map(|i| i.logical_size())
            .unwrap_or((1024.0, 640.0));
        let x = (bg.win_x as f32).clamp(0.0, (tw - 1024.0).max(0.0));
        let y = (bg.win_y as f32).clamp(0.0, (th - 640.0).max(0.0));
        SpriteDraw {
            image: bg.name.clone(),
            rect: [0.0, 0.0, 1024.0, 640.0],
            uv: [x / tw, y / th, 1024.0_f32.min(tw) / tw, 640.0_f32.min(th) / th],
            color: [alpha, alpha, alpha, alpha],
        }
    }
}
fn scale_frame_draw(
    draw: &mut FrameDraw,
    mask: Option<&mut MaskTransitionDraw>,
    inset: f32,
) {
    let inset = inset.clamp(0.0, 319.0);
    let sx = (1024.0 - 2.0 * inset).max(1.0) / 1024.0;
    let sy = (640.0 - 2.0 * inset).max(1.0) / 640.0;
    let scale = |rect: &mut [f32; 4]| {
        rect[0] = inset + rect[0] * sx;
        rect[1] = inset + rect[1] * sy;
        rect[2] *= sx;
        rect[3] *= sy;
    };
    for sprite in draw.sprites.iter_mut().chain(draw.sprites2.iter_mut()) {
        scale(&mut sprite.rect);
    }
    for glyph in draw.glyphs.iter_mut().chain(draw.glyphs2.iter_mut()) {
        scale(&mut glyph.rect);
    }
    for effect in &mut draw.effects {
        effect.x = (inset + effect.x as f32 * sx).round() as i32;
        effect.y = (inset + effect.y as f32 * sy).round() as i32;
    }
    if let Some(mask) = mask {
        scale(&mut mask.rect);
    }
}
fn translate_frame_draw(
    draw: &mut FrameDraw,
    mask: Option<&mut MaskTransitionDraw>,
    dx: f32,
    dy: f32,
) {
    for sprite in draw.sprites.iter_mut().chain(draw.sprites2.iter_mut()) {
        sprite.rect[0] += dx;
        sprite.rect[1] += dy;
    }
    for glyph in draw.glyphs.iter_mut().chain(draw.glyphs2.iter_mut()) {
        glyph.rect[0] += dx;
        glyph.rect[1] += dy;
    }
    for effect in &mut draw.effects {
        effect.x += dx.round() as i32;
        effect.y += dy.round() as i32;
    }
    if let Some(mask) = mask {
        mask.rect[0] += dx;
        mask.rect[1] += dy;
    }
}
fn sdraw(name: &str) -> SpriteDraw {
    SpriteDraw::full(name)
}
fn panel_asset(mode: i64) -> &'static str {
    match mode {
        1 => "msgPanel.png",
        2 => "fullPanel.png",
        3 | 6 => "BLACK1024x75.png",
        4 => "fullPanel2.png",
        5 => "TRANS1024x75.png",
        _ => "",
    }
}
fn panel_rect_for_mode(mode: i64) -> [f32; 4] {
    match mode {
        1 => [0.0, 457.0, 1024.0, 183.0],
        2 | 4 => [0.0, 0.0, 1024.0, 640.0],
        3 | 5 | 6 => [0.0, 565.0, 1024.0, 75.0],
        _ => [0.0; 4],
    }
}
fn panel_text_visible(mode: i64) -> bool {
    mode != 0
}
#[allow(clippy::too_many_arguments)]
fn push_panel_layer(
    atlas: &mut GlyphAtlas,
    resources: &dyn ResourceStore,
    frame_no: u64,
    vp: &Viewport,
    sprites: &mut Vec<SpriteDraw>,
    glyphs: &mut Vec<GlyphQuad>,
    message: Option<&crate::scene::MessageState>,
    mode: i64,
    alpha: f32,
    awaiting: bool,
) {
    if !panel_text_visible(mode) || alpha <= 0.0 {
        return;
    }
    let (
        panel_img,
        panel_rect,
        msg_box,
        name_box,
    ): (&str, [f32; 4], &TextBox, Option<&TextBox>) = match mode {
        1 => {
            (
                panel_asset(1),
                panel_rect_for_mode(1),
                &MSG_BOX_PANEL1,
                Some(&NAME_BOX_PANEL1),
            )
        }
        2 => {
            (panel_asset(2), panel_rect_for_mode(2), &MSG_BOX_FULL, Some(&NAME_BOX_FULL))
        }
        3 => (panel_asset(3), panel_rect_for_mode(3), &MSG_BOX_BAR, Some(&NAME_BOX_BAR)),
        5 => (panel_asset(5), panel_rect_for_mode(5), &MSG_BOX_BAR, Some(&NAME_BOX_BAR)),
        4 => (panel_asset(4), panel_rect_for_mode(4), &MSG_BOX_FULL2, None),
        6 => (panel_asset(6), panel_rect_for_mode(6), &MSG_BOX_BAR, Some(&NAME_BOX_BAR)),
        _ => return,
    };
    sprites
        .push(SpriteDraw {
            image: panel_img.into(),
            rect: panel_rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [alpha; 4],
        });
    let Some(msg) = message else { return };
    let name = msg.speaker.as_deref().and_then(|s| speaker_display_for_panel(s, mode));
    if let (Some(box_), Some(shown)) = (name_box, name.as_deref()) {
        let mut out = layout_ex(
            atlas,
            vp,
            shown,
            box_,
            22.0,
            10.0,
            [1.0; 4],
            false,
            0,
            None,
        );
        for q in &mut out.quads {
            q.color[3] *= alpha;
        }
        glyphs.extend(out.quads);
    }
    let first_line = usize::from(name.is_some() && mode == 1);
    let mut body = layout_ex(
        atlas,
        vp,
        &msg.body,
        msg_box,
        22.0,
        10.0,
        [1.0; 4],
        true,
        first_line,
        Some(msg.revealed),
    );
    for q in &mut body.quads {
        q.color[3] *= alpha;
    }
    glyphs.extend(body.quads);
    if msg.done() && awaiting {
        if let Some((pen_x, line_top)) = body.cursor {
            const CURSOR_FRAME: f32 = 22.0;
            let strip = if (mode == 2 || mode == 4) && !msg.is_read {
                "pauseAnimeNextPage.png"
            } else {
                "pauseAnime.png"
            };
            let strip_w = resources
                .request_image(strip)
                .ready()
                .map(|image| image.logical_w)
                .unwrap_or(264.0);
            let frames = (strip_w / CURSOR_FRAME).max(1.0) as u64;
            let x = ((pen_x + 5.0) as i32 & !1) as f32;
            let y = line_top + 31.0 - CURSOR_FRAME + 2.0;
            let f = ((frame_no / 10) % frames) as f32;
            sprites
                .push(SpriteDraw {
                    image: strip.into(),
                    rect: [x, y, CURSOR_FRAME, CURSOR_FRAME],
                    uv: [f * CURSOR_FRAME / strip_w, 0.0, CURSOR_FRAME / strip_w, 1.0],
                    color: [alpha; 4],
                });
        }
    }
}
fn config_slider_set(settings: &Rc<RefCell<Settings>>, cmd: u8, v: i64) {
    let mut s = settings.borrow_mut();
    match cmd {
        1 => s.message_speed_tbr = v,
        2 => s.message_speed_read = v,
        3 => s.message_speed_autoplay = v,
        17 => s.volume_bgm = v,
        18 => s.volume_voice = v,
        19 => s.volume_se = v,
        _ => {}
    }
}
fn background_progress_allowed(focused: bool, settings: &Rc<RefCell<Settings>>) -> bool {
    focused || settings.borrow().progress_in_background != 0
}
fn stop_voice_on_advance(
    settings: &Rc<RefCell<Settings>>,
    audio: &Rc<RefCell<dyn AudioBackend>>,
) {
    if settings.borrow().stop_voice_at_next != 0 {
        audio.borrow_mut().stop_voice();
    }
}
fn config_widget(
    settings: &Rc<RefCell<Settings>>,
    audio: &Rc<RefCell<dyn AudioBackend>>,
    cmd: u8,
) {
    let mut s = settings.borrow_mut();
    match cmd {
        6 => s.play_mode = 1,
        7 => s.play_mode = 0,
        10 => s.skip_showing = (s.skip_showing == 0) as i64,
        11 => s.screen_effect = (s.screen_effect == 0) as i64,
        12 => s.shadow = (s.shadow == 0) as i64,
        13 => s.animation = (s.animation == 0) as i64,
        14 => s.backlog_voice_playback = (s.backlog_voice_playback == 0) as i64,
        15 => s.stop_voice_at_next = (s.stop_voice_at_next == 0) as i64,
        16 => s.progress_in_background = (s.progress_in_background == 0) as i64,
        20 => s.mute_bgm = !s.mute_bgm,
        21 => s.mute_voice = !s.mute_voice,
        22 => s.mute_se = !s.mute_se,
        23..=25 => {
            let (name, vol, channel) = match cmd {
                23 => ("BGMtest.wav", s.volume_bgm, AuxChannel::TestBgm),
                24 => ("VOICEtest.wav", s.volume_voice, AuxChannel::TestVoice),
                _ => ("SEtest.wav", s.volume_se, AuxChannel::TestSe),
            };
            drop(s);
            audio.borrow_mut().play_aux(name, vol, channel);
            return;
        }
        26..=33 => {
            let i = (cmd - 26) as usize;
            s.voice_chars[i] = !s.voice_chars[i];
        }
        _ => {}
    }
}
fn in_rect(x: f32, y: f32, r: [f32; 4]) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}
fn viewer_skip_missing(resources: &dyn ResourceStore, g: &mut CgGallery) {
    for _ in 0..1000 {
        let Some(v) = &mut g.viewer else { return };
        if v.closing {
            return;
        }
        let missing = v
            .current()
            .map(|name| {
                matches!(
                    resources.request_image(name), crate
                    ::resource::ResourceState::Missing | crate
                    ::resource::ResourceState::Failed(_)
                )
            })
            .unwrap_or(true);
        if !missing {
            return;
        }
        v.advance();
    }
}
fn gallery_scroll(resources: &dyn ResourceStore, g: &mut CgGallery, dx: i32, dy: i32) {
    let Some(v) = &mut g.viewer else { return };
    let Some(name) = v.current().map(|s| s.to_string()) else {
        return;
    };
    let (tw, th) = resources
        .request_image(&name)
        .ready()
        .map(|i| (i.logical_w.round() as i32, i.logical_h.round() as i32))
        .unwrap_or((1024, 640));
    v.scroll(dx, dy);
    v.win_x = v.win_x.clamp(0, (tw - 1024).max(0));
    v.win_y = v.win_y.clamp(0, (th - 640).max(0));
}
fn draw_digits(
    sprites: &mut Vec<SpriteDraw>,
    font: &str,
    gw: f32,
    gh: f32,
    value: u32,
    at: [f32; 2],
    color: [f32; 4],
) {
    let strip_w = gw * 11.0;
    for (i, d) in [value / 10 % 10, value % 10].iter().enumerate() {
        sprites
            .push(SpriteDraw {
                image: font.into(),
                rect: [at[0] + gw * i as f32, at[1], gw, gh],
                uv: [gw * *d as f32 / strip_w, 0.0, gw / strip_w, 1.0],
                color,
            });
    }
}
fn title_item_enabled(
    progress: &std::collections::BTreeMap<String, String>,
    item: usize,
) -> bool {
    match TITLE_MENU[item].0 {
        "Memories" => global_flag(progress, "eden_D001"),
        "Mosaic" => false,
        _ => true,
    }
}
fn global_flag(
    progress: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> bool {
    progress
        .get(name)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map(|v| v != 0)
        .unwrap_or(false)
}
fn visible_title_items() -> usize {
    TITLE_MENU.iter().rposition(|(_, vis)| *vis).map(|i| i + 1).unwrap_or(1)
}
#[cfg(not(target_arch = "wasm32"))]
fn load_progress_from(
    storage: &dyn StorageBackend,
) -> std::collections::BTreeMap<String, String> {
    let Ok(Some(bytes)) = storage.read("system.dat") else {
        return std::collections::BTreeMap::new();
    };
    eden_formats::system_dat::decode(&bytes)
        .map_err(|e| log::warn!("system.dat 读取失败: {e:?}"))
        .unwrap_or_default()
}
fn persist_settings_to(storage: &dyn StorageBackend, settings: &Settings) {
    if let Err(error) = storage.write("settings.tsv", &settings.to_bytes()) {
        log::warn!("设置持久化失败: {error}");
    }
}
fn queue_config_screen_mode(
    current: ScreenMode,
    pending: &mut Option<ScreenMode>,
    requests: &mut VecDeque<PlatformRequest>,
    want_fullscreen: bool,
) {
    let target = if want_fullscreen {
        ScreenMode::Borderless
    } else {
        ScreenMode::Windowed
    };
    if current == target {
        return;
    }
    *pending = Some(target);
    requests
        .push_back(PlatformRequest::SetScreenMode {
            mode: target,
        });
}
fn queue_exit(
    quit: &mut bool,
    pending_exit: &mut bool,
    requests: &mut VecDeque<PlatformRequest>,
) {
    if *pending_exit || *quit {
        return;
    }
    *pending_exit = true;
    requests.push_back(PlatformRequest::QueryWindowGeometry);
}
fn title_hit_test(x: f32, y: f32) -> Option<usize> {
    TITLE_HIT.iter().position(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
}
fn memories_hit_test(x: f32, y: f32) -> Option<usize> {
    MEMORIES_HIT
        .iter()
        .position(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
        .filter(|&i| MEMORIES_MENU[i].1)
}
fn menu_release_commit(
    pressed: Option<usize>,
    released: Option<usize>,
) -> Option<usize> {
    match (pressed, released) {
        (Some(pressed), Some(released)) if pressed == released => Some(released),
        _ => None,
    }
}
fn gallery_wheel_steps(dx: f32, dy: f32) -> (i32, i32) {
    ((-dx * 50.0) as i32, (-dy * 50.0) as i32)
}
fn encode_gray_png(data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    out
}

