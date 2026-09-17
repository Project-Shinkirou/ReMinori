use crate::audio::{parse_vol_pan, AudioBackend, FadeTicks};
use crate::cgmode::CgSeen;
use crate::read_history::ReadHistory;
use crate::resource::ResourceStore;
use crate::scene::{MessageState, Scene};
use crate::settings::Settings;
use crate::storage::StorageBackend;
use eden_formats::script::{
    Arg, Effect, Message, Movie, Panel, PlayBgm, PlaySe, Raw, Stage, Transition,
};
use eden_vm::Host;
use std::cell::RefCell;
use std::rc::Rc;
pub struct EngineHost {
    pub resources: Rc<dyn ResourceStore>,
    pub scene: Rc<RefCell<Scene>>,
    pub audio: Rc<RefCell<dyn AudioBackend>>,
    pub cg_seen: Rc<RefCell<CgSeen>>,
    pub settings: Rc<RefCell<Settings>>,
    pub read_history: Rc<RefCell<ReadHistory>>,
    pub storage: Rc<dyn StorageBackend>,
}
impl EngineHost {
    pub fn new<R: ResourceStore + 'static>(
        resources: Rc<R>,
        scene: Rc<RefCell<Scene>>,
        audio: Rc<RefCell<dyn AudioBackend>>,
        cg_seen: Rc<RefCell<CgSeen>>,
        settings: Rc<RefCell<Settings>>,
    ) -> Self {
        let resources: Rc<dyn ResourceStore> = resources;
        Self::with_read_history(
            resources,
            scene,
            audio,
            cg_seen,
            settings,
            Rc::new(RefCell::new(ReadHistory::default())),
            Rc::new(crate::storage::MemoryStorage::default()),
        )
    }
    pub fn with_read_history(
        resources: Rc<dyn ResourceStore>,
        scene: Rc<RefCell<Scene>>,
        audio: Rc<RefCell<dyn AudioBackend>>,
        cg_seen: Rc<RefCell<CgSeen>>,
        settings: Rc<RefCell<Settings>>,
        read_history: Rc<RefCell<ReadHistory>>,
        storage: Rc<dyn StorageBackend>,
    ) -> Self {
        EngineHost {
            resources,
            scene,
            audio,
            cg_seen,
            settings,
            read_history,
            storage,
        }
    }
    fn persist_read_history(&self) {
        if let Err(error) = self
            .storage
            .write("readflags.bin", &self.read_history.borrow().to_bytes())
        {
            log::warn!("已读表持久化失败: {error}");
        }
    }
    fn play_voice_resolved(&mut self, name: &str) {
        self.audio.borrow_mut().stop_voice();
        let tail = name.rsplit('\\').next().unwrap_or(name);
        let s = self.settings.borrow();
        let b = tail.as_bytes();
        let key_len = if b.len() >= 4 && b[3] == b'-' { 3 } else { 2 };
        if b.len() >= key_len && !s.voice_flag(&tail[..key_len].to_ascii_lowercase()) {
            return;
        }
        let (resolved, vol, pan) = parse_vol_pan(tail, s.volume_voice);
        let mute = s.mute_voice;
        drop(s);
        if !mute {
            self.audio.borrow_mut().play_voice(&resolved, vol, pan);
        }
    }
}
impl Host for EngineHost {
    fn message(&mut self, msg: &Message, is_read: bool) {
        let previous = self
            .scene
            .borrow()
            .message
            .as_ref()
            .map(|message| message.message_no);
        if let Some(previous) = previous.filter(|&no| no != msg.no) {
            self.read_history.borrow_mut().mark(previous);
            self.persist_read_history();
        }
        let body = msg.body_text();
        let visible = crate::render::text::visible_chars(&body);
        let speaker = msg.speaker.as_ref().map(|s| s.token());
        let voice = msg.voice.as_ref().map(|v| v.name.clone());
        let mut scene = self.scene.borrow_mut();
        let append = scene
            .message
            .as_ref()
            .is_some_and(|current| current.message_no == msg.no);
        if append {
            if let Some(current) = &mut scene.message {
                current.append_text(&body, visible, speaker, voice);
            }
        } else {
            scene.message = Some(MessageState {
                message_no: msg.no,
                speaker,
                revealed: 0,
                char_timer: 0,
                visible_total: visible,
                is_read,
                char_interval: if is_read {
                    self.settings.borrow().message_speed_read
                } else {
                    self.settings.borrow().message_speed_tbr
                }
                    .clamp(0, 100) as u32 / crate::scene::MESSAGE_SPEED_DIV,
                voice,
                body,
            });
        }
        drop(scene);
        if let Some(v) = &msg.voice {
            self.play_voice_resolved(&v.name);
        }
    }
    fn message_complete(&self) -> bool {
        self.scene.borrow().message.as_ref().is_none_or(MessageState::done)
    }
    fn message_is_read(&self, message_no: u32) -> bool {
        self.read_history.borrow().contains(message_no)
    }
    fn voice_playing(&self) -> bool {
        self.audio.borrow().voice_playing()
    }
    fn stage(&mut self, s: &Stage) {
        let name = s.file.to_string();
        {
            let mut seen = self.cg_seen.borrow_mut();
            seen.insert(&name);
            if let Err(error) = self.storage.write("cgseen.tsv", &seen.to_bytes()) {
                log::warn!("CG 解锁集持久化失败: {error}");
            }
        }
        self.scene.borrow_mut().stage(&name, arg_i(&s.x), arg_i(&s.y));
    }
    fn transition(&mut self, t: &Transition) {
        let mask = match &t.mask {
            Arg::Star | Arg::Int(_) => None,
            Arg::Word(w) => Some(w.clone()),
        };
        self.scene
            .borrow_mut()
            .set_transition(arg_i(&t.kind), mask, arg_i(&t.frames).max(0) as u32);
    }
    fn scroll_vh(&mut self, vertical: bool, a: i64, b: i64) {
        self.scene.borrow_mut().scroll_vh(vertical, a, b);
    }
    fn scroll(&mut self, x: i64, y: i64, n: i64) {
        self.scene.borrow_mut().scroll_xy(x, y, n);
    }
    fn end_scroll(&mut self, flag: bool) -> bool {
        self.scene.borrow_mut().end_scroll(flag)
    }
    fn shake_screen(&mut self, mode: &str, a: i64, b: i64) {
        self.scene.borrow_mut().shake_screen(mode, a, b);
    }
    fn effect(&mut self, e: &Effect) {
        match e.name.to_string().as_str() {
            "Firefly" if e
                .name2
                .as_ref()
                .is_some_and(|name| name.to_string() == "Firefly") => {
                let count = e.ints.first().map(arg_i).unwrap_or(-1);
                let life = e.ints.get(1).map(arg_i).unwrap_or(-1);
                self.scene.borrow_mut().start_firefly(count, life);
            }
            "end" => self.scene.borrow_mut().end_effect(),
            _ => {}
        }
    }
    fn play_bgm(&mut self, p: &PlayBgm) {
        let fade_in = positive_ticks(p.fade_in());
        let fade_out = positive_ticks(p.fade_out());
        if p.file == Arg::Star {
            self.audio.borrow_mut().stop_bgm(fade_out);
        } else {
            let s = self.settings.borrow();
            let (name, _, pan) = parse_vol_pan(&p.file.to_string(), 100);
            let vol = p.volume() * s.volume_bgm / 100;
            let mute = s.mute_bgm;
            drop(s);
            self.audio
                .borrow_mut()
                .play_bgm(&name, vol, pan, true, fade_in, fade_out, mute);
        }
    }
    fn play_se(&mut self, p: &PlaySe) {
        let looping = p.looping();
        let fade_in = positive_ticks(p.fade_in());
        let fade_out = positive_ticks(p.fade_out());
        if p.file == Arg::Star {
            self.audio.borrow_mut().stop_se(p.se2, fade_out);
            return;
        }
        let s = self.settings.borrow();
        let (name, vol, pan) = parse_vol_pan(&p.file.to_string(), s.volume_se);
        let mute = s.mute_se;
        drop(s);
        self.audio
            .borrow_mut()
            .play_se(&name, vol, pan, p.se2, looping, fade_in, fade_out, mute);
    }
    fn play_voice(&mut self, name: &str, _rest: &[Arg]) {
        if name != "*" {
            self.play_voice_resolved(name);
        } else {
            self.audio.borrow_mut().stop_voice();
        }
    }
    fn movie(&mut self, m: &Movie) {
        let request = crate::scene::MovieRequest {
            file: m.file.to_string(),
            width: m.w.as_ref().map_or(1024, |arg| arg.script_int(1024)) as i32,
            height: m.h.as_ref().map_or(640, |arg| arg.script_int(640)) as i32,
            skip_allowed: m.skip.as_ref().and_then(Arg::as_flag).unwrap_or(true),
            x: m.x.as_ref().map_or(0, |arg| arg.script_int(0)) as i32,
            y: m.y.as_ref().map_or(0, |arg| arg.script_int(0)) as i32,
        };
        log::info!(
            "movie: {} rect=({},{},{},{}) skip={}", request.file, request.x, request.y,
            request.width, request.height, request.skip_allowed
        );
        self.scene.borrow_mut().movie = Some(crate::scene::MovieState::Pending(request));
    }
    fn panel(&mut self, p: &Panel) -> bool {
        let Some(mode) = p.args.first() else {
            return false;
        };
        let speed = p.args.get(1).map_or(-1, |a| a.script_int(-1));
        self.scene.borrow_mut().set_panel(arg_i(mode), speed)
    }
    fn pragma(&mut self, name: &str) {
        log::debug!("pragma: {name}");
    }
    fn end(&mut self) {
        if let Some(no) = self
            .scene
            .borrow()
            .message
            .as_ref()
            .map(|message| message.message_no)
        {
            self.read_history.borrow_mut().mark(no);
            self.persist_read_history();
        }
        log::info!("脚本结束（.end）");
    }
    fn load_script(&mut self, name: &str) -> Option<eden_formats::ScriptIr> {
        let key = name.to_ascii_lowercase();
        self.resources
            .script(&key)
            .map(|ir| eden_formats::ScriptIr {
                nodes: ir.nodes.clone(),
            })
    }
    fn unknown(&mut self, raw: &Raw) {
        log::warn!("未知命令跳过: {}", raw.text);
    }
}
fn arg_i(a: &Arg) -> i64 {
    a.to_string().parse().unwrap_or(0)
}
fn positive_ticks(n: i64) -> FadeTicks {
    (n >= 1).then_some(n)
}
