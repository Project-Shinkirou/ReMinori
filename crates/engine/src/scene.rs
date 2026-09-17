#[derive(Debug, Clone, PartialEq)]
pub struct BgSlot {
    pub name: String,
    pub win_x: i64,
    pub win_y: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TrSpec {
    pub kind: i64,
    pub mask: Option<String>,
    pub frames: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTr {
    pub spec: TrSpec,
    pub t: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PanelTr {
    pub from: i64,
    pub to: i64,
    pub t: u32,
    pub speed: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct MessageState {
    pub message_no: u32,
    pub speaker: Option<String>,
    pub body: String,
    pub voice: Option<String>,
    pub revealed: usize,
    pub char_timer: u32,
    pub visible_total: usize,
    pub is_read: bool,
    pub char_interval: u32,
}
impl MessageState {
    pub fn append_text(
        &mut self,
        body: &str,
        visible: usize,
        speaker: Option<String>,
        voice: Option<String>,
    ) {
        self.body.push_str(body);
        self.visible_total = self.visible_total.saturating_add(visible);
        if speaker.is_some() {
            self.speaker = speaker;
        }
        self.voice = voice;
    }
    pub fn done(&self) -> bool {
        self.revealed >= self.visible_total
    }
    fn type_tick(&mut self) {
        if self.done() {
            return;
        }
        if self.char_timer > 0 {
            self.char_timer -= 1;
            return;
        }
        self.revealed += 1;
        self.char_timer = self.char_interval;
    }
    pub fn reveal_all(&mut self) {
        self.revealed = self.visible_total;
        self.char_timer = 0;
    }
}
pub const MESSAGE_SPEED_DEFAULT: u32 = 50;
pub const MESSAGE_SPEED_DIV: u32 = 5;
pub const EYE_SEQ: [&[(i16, u16)]; 6] = [
    &[
        (0, 40),
        (1, 40),
        (2, 80),
        (1, 40),
        (0, 400),
        (1, 40),
        (2, 80),
        (1, 40),
        (0, 3600),
        (1, 40),
        (2, 80),
        (1, 40),
        (0, 1200),
        (-1, 0),
    ],
    &[
        (0, 10),
        (1, 20),
        (2, 40),
        (1, 20),
        (0, 200),
        (1, 20),
        (2, 40),
        (1, 20),
        (0, 1800),
        (1, 20),
        (2, 40),
        (1, 20),
        (0, 600),
        (-1, 0),
    ],
    &[
        (0, 10),
        (1, 10),
        (2, 20),
        (1, 10),
        (0, 100),
        (1, 10),
        (2, 20),
        (1, 10),
        (0, 900),
        (1, 10),
        (2, 20),
        (1, 10),
        (0, 300),
        (-1, 0),
    ],
    &[
        (0, 5),
        (1, 5),
        (2, 10),
        (1, 5),
        (0, 100),
        (1, 5),
        (2, 10),
        (1, 5),
        (0, 450),
        (1, 5),
        (2, 10),
        (1, 5),
        (0, 150),
        (-1, 0),
    ],
    &[
        (0, 3),
        (1, 3),
        (2, 7),
        (1, 3),
        (0, 33),
        (1, 3),
        (2, 7),
        (1, 3),
        (0, 225),
        (1, 3),
        (2, 7),
        (1, 3),
        (0, 75),
        (-1, 0),
    ],
    &[(0, 3), (1, 3), (2, 3), (1, 3), (-1, 0)],
];
pub const EYE_OPEN_RAND_MOD: u32 = 500;
pub const EYE_OPEN_RAND_BASE: u32 = 50;
pub const EYE_WORDS: [&str; 3] = ["開き目", "中目", "閉じ目"];
pub const MOUTH_WORDS: [&str; 3] = ["閉じ口", "中口", "開き口"];
#[derive(Debug, Clone, PartialEq)]
pub struct EyeWalker {
    pub seq_sel: usize,
    pub seq_idx: usize,
    pub countdown: i32,
    pub state: i16,
    started: bool,
}
impl EyeWalker {
    pub fn new(seq_sel: usize) -> Self {
        EyeWalker {
            seq_sel,
            seq_idx: 0,
            countdown: 1,
            state: 0,
            started: false,
        }
    }
    pub fn tick(&mut self) -> Option<i16> {
        if !self.started {
            self.started = true;
            self.countdown = 1;
        }
        self.countdown -= 1;
        if self.countdown > 0 {
            return None;
        }
        let seq = EYE_SEQ[self.seq_sel];
        loop {
            let (s, t) = seq[self.seq_idx];
            if s < 0 {
                self.seq_idx = 0;
                continue;
            }
            self.state = s;
            self.countdown = t as i32;
            if s == 0 && self.seq_sel < 5 {
                self.countdown
                    += (rand_u32() % EYE_OPEN_RAND_MOD + EYE_OPEN_RAND_BASE) as i32;
            }
            break;
        }
        self.seq_idx += 1;
        Some(self.state)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct MouthState {
    pub state: u8,
    last_seq: u64,
    prev_level: u32,
}
impl Default for MouthState {
    fn default() -> Self {
        Self::new()
    }
}
impl MouthState {
    pub fn new() -> Self {
        MouthState {
            state: 0,
            last_seq: 0,
            prev_level: 0,
        }
    }
    pub fn tick(&mut self, seq: u64, playing: bool, level: u32) -> Option<u8> {
        if self.last_seq == seq {
            return None;
        }
        self.last_seq = seq;
        let next = if playing && level >= 0x320 {
            if level > self.prev_level {
                Some(if self.state != 0 { 2 } else { 1 })
            } else if self.state == 1 && level < self.prev_level / 2 {
                Some(0)
            } else {
                None
            }
        } else if self.state != 2 {
            Some(0)
        } else {
            None
        };
        self.prev_level = level;
        match next {
            Some(s) => {
                self.state = s;
                Some(s)
            }
            None => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct CgCharAnim {
    pub name: String,
    pub eye: EyeWalker,
    pub mouth: MouthState,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CgAnim {
    pub png: String,
    pub chars: Vec<CgCharAnim>,
    pub tick: u64,
}
impl CgAnim {
    pub fn new(png: &str, char_names: Vec<String>) -> Self {
        CgAnim {
            png: png.to_ascii_lowercase(),
            chars: char_names
                .into_iter()
                .map(|name| CgCharAnim {
                    name,
                    eye: EyeWalker::new(2),
                    mouth: MouthState::new(),
                })
                .collect(),
            tick: 0,
        }
    }
    pub fn tick(&mut self, speaker: Option<&str>, voice: (bool, u32)) {
        self.tick += 1;
        let spk = speaker
            .map(|s| s.trim_matches(['@', '#']).replace(['　', ' ', '\t'], ""))
            .unwrap_or_default();
        for c in &mut self.chars {
            c.eye.tick();
            if !spk.is_empty() && c.name == spk {
                c.mouth.tick(self.tick, voice.0, voice.1);
            } else if c.mouth.state != 0 {
                c.mouth.state = 0;
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct MovieRequest {
    pub file: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub skip_allowed: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub enum MovieState {
    Pending(MovieRequest),
    Playing(MovieRequest),
}
#[derive(Debug, Clone, PartialEq)]
pub enum ScrollAnim {
    V { src: i64, target: i64, speed: i64, t: u32 },
    H { src: i64, target: i64, speed: i64, t: u32 },
    Xy { src_x: i64, src_y: i64, tx: i64, ty: i64, speed: i64, t: u32 },
}
impl ScrollAnim {
    fn t(&self) -> u32 {
        match self {
            ScrollAnim::V { t, .. }
            | ScrollAnim::H { t, .. }
            | ScrollAnim::Xy { t, .. } => *t,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShakeMode {
    Random,
    Zoom,
    Vertical,
    Horizontal,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Shake {
    mode: ShakeMode,
    pub amt: i64,
    pub ticks_left: i64,
    dir_t: u32,
    pub dx: f32,
    pub dy: f32,
    zoomed: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FireflyParticle {
    pub points: [[f64; 2]; 7],
    pub texture: usize,
    pub age: i64,
    pub life: i64,
    pub x: i32,
    pub y: i32,
    pub alpha: u16,
}
impl FireflyParticle {
    fn new(base_life: i64) -> Self {
        let mut p = FireflyParticle {
            points: [[0.0; 2]; 7],
            texture: 0,
            age: 0,
            life: 1,
            x: 0,
            y: 0,
            alpha: 0,
        };
        p.reset(base_life);
        p
    }
    fn reset(&mut self, base_life: i64) {
        let base_life = base_life.max(1);
        for point in &mut self.points {
            point[0] = (rand_u32() % 2048) as f64 - 512.0;
            point[1] = (rand_u32() % 1280) as f64 - 320.0;
        }
        self.texture = (rand_u32() % 3) as usize;
        self.age = 0;
        self.alpha = 0;
        self.life = base_life + (rand_u32() % base_life as u32) as i64 - base_life / 2;
        self.x = self.points[0][0] as i32;
        self.y = self.points[0][1] as i32;
    }
    fn tick(&mut self, elapsed: i64) {
        self.age += elapsed;
        if self.age < self.life {
            self.set_curve(self.age as f64 / self.life as f64);
        } else {
            self.set_curve(1.0);
            let previous_life = self.life;
            self.reset(previous_life);
        }
    }
    fn set_curve(&mut self, t: f64) {
        const BINOMIAL: [f64; 7] = [1.0, 6.0, 15.0, 20.0, 15.0, 6.0, 1.0];
        let mut x = 0.0;
        let mut y = 0.0;
        for (i, point) in self.points.iter().enumerate() {
            let weight = BINOMIAL[i] * t.powi(i as i32) * (1.0 - t).powi((6 - i) as i32);
            x += weight * point[0];
            y += weight * point[1];
        }
        self.x = x.trunc() as i32;
        self.y = y.trunc() as i32;
        self.alpha = if t < 0.255 {
            (t * 1000.0).trunc().clamp(0.0, 255.0) as u16
        } else if t <= 0.755 {
            255
        } else {
            ((1.0 - t) * 1000.0).trunc().clamp(0.0, 255.0) as u16
        };
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Firefly {
    pub particles: Vec<FireflyParticle>,
    pub max_count: usize,
    pub base_life: i64,
    pub alpha: u16,
    fade_accumulator: u32,
    clock: u32,
    last_update: Option<u32>,
}
impl Firefly {
    pub fn new(count: i64, life: i64) -> Self {
        let count = count.max(0) as usize;
        let base_life = life.unsigned_abs().max(1) as i64;
        Firefly {
            particles: (0..count).map(|_| FireflyParticle::new(base_life)).collect(),
            max_count: count,
            base_life,
            alpha: 0,
            fade_accumulator: 0,
            clock: 0,
            last_update: None,
        }
    }
    pub fn tick(&mut self) {
        const DRAW_INTERVAL: u32 = 10;
        self.clock = self.clock.wrapping_add(1);
        let elapsed = match self.last_update {
            None => 1,
            Some(last) => {
                let elapsed = self.clock.abs_diff(last);
                if elapsed < DRAW_INTERVAL {
                    return;
                }
                if elapsed > 20 { DRAW_INTERVAL } else { elapsed }
            }
        };
        self.last_update = Some(self.clock);
        self.fade_accumulator = self.fade_accumulator.saturating_add(elapsed);
        let step = (self.fade_accumulator >> 4) as u16;
        self.alpha = self.alpha.saturating_add(step).min(256);
        for particle in &mut self.particles {
            particle.tick(elapsed as i64);
        }
    }
}
pub const FIREFLY_IMAGES: [&str; 3] = ["FireflyS.png", "FireflyM.png", "FireflyL.png"];
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub bg: Option<BgSlot>,
    pub prev_bg: Option<BgSlot>,
    pub tr: TrSpec,
    pub active_tr: Option<ActiveTr>,
    pub panel_mode: i64,
    pub panel_tr: Option<PanelTr>,
    pub message: Option<MessageState>,
    pub movie: Option<MovieState>,
    pub scroll: Option<ScrollAnim>,
    pub shake: Option<Shake>,
    pub firefly: Option<Firefly>,
    pub cg_anim: Option<CgAnim>,
    pub msg_type_paused: bool,
    pub effect_tick: bool,
}
impl Default for Scene {
    fn default() -> Self {
        Scene {
            bg: None,
            prev_bg: None,
            tr: TrSpec {
                kind: 0,
                mask: None,
                frames: 0,
            },
            active_tr: None,
            panel_mode: 0,
            panel_tr: None,
            message: None,
            movie: None,
            scroll: None,
            shake: None,
            firefly: None,
            cg_anim: None,
            msg_type_paused: false,
            effect_tick: true,
        }
    }
}
impl Scene {
    pub fn set_transition(&mut self, kind: i64, mask: Option<String>, frames: u32) {
        self.tr = TrSpec { kind, mask, frames };
    }
    pub fn set_panel(&mut self, mode: i64, speed: i64) -> bool {
        if self.panel_mode == mode {
            return false;
        }
        let speed = if speed <= 0 { 16 } else { speed as u32 };
        let from = self.panel_mode;
        self.panel_mode = mode;
        self.panel_tr = Some(PanelTr {
            from,
            to: mode,
            t: 0,
            speed,
        });
        true
    }
    pub fn panel_alpha(&self) -> f32 {
        self.panel_tr
            .as_ref()
            .map(|tr| (tr.t.saturating_mul(tr.speed).min(255) as f32) / 255.0)
            .unwrap_or(1.0)
    }
    pub fn finish_panel_transition(&mut self) {
        self.panel_tr = None;
    }
    fn win(&self) -> (i64, i64) {
        self.bg.as_ref().map(|b| (b.win_x, b.win_y)).unwrap_or((0, 0))
    }
    pub fn stage(&mut self, name: &str, x: i64, y: i64) {
        self.finish_scroll();
        self.cg_anim = None;
        let new_bg = BgSlot {
            name: name.to_string(),
            win_x: x,
            win_y: y,
        };
        let old = self.bg.take().or_else(|| self.prev_bg.take());
        if self.tr.frames > 0 && old.as_ref().is_some_and(|o| o.name != new_bg.name) {
            self.prev_bg = old;
            self.active_tr = Some(ActiveTr {
                spec: self.tr.clone(),
                t: 0,
            });
        } else {
            self.prev_bg = None;
            self.active_tr = None;
        }
        self.bg = Some(new_bg);
    }
    pub fn scroll_vh(&mut self, vertical: bool, target: i64, speed: i64) {
        let (x, y) = self.win();
        self.scroll = Some(
            if vertical {
                ScrollAnim::V {
                    src: y,
                    target,
                    speed,
                    t: 0,
                }
            } else {
                ScrollAnim::H {
                    src: x,
                    target,
                    speed,
                    t: 0,
                }
            },
        );
    }
    pub fn scroll_xy(&mut self, tx: i64, ty: i64, speed: i64) {
        let (x, y) = self.win();
        self.scroll = Some(ScrollAnim::Xy {
            src_x: x,
            src_y: y,
            tx,
            ty,
            speed,
            t: 0,
        });
    }
    pub fn end_scroll(&mut self, flag: bool) -> bool {
        if self.scroll.is_none() {
            return false;
        }
        if flag {
            self.finish_scroll();
        }
        true
    }
    pub fn finish_scroll(&mut self) {
        if let Some(sa) = self.scroll.take() {
            let (bg_x, bg_y) = self.win();
            let (x, y) = match sa {
                ScrollAnim::V { target, .. } => (bg_x, target),
                ScrollAnim::H { target, .. } => (target, bg_y),
                ScrollAnim::Xy { tx, ty, .. } => (tx, ty),
            };
            if let Some(bg) = &mut self.bg {
                bg.win_x = x;
                bg.win_y = y;
            }
        }
    }
    pub fn shake_screen(&mut self, mode: &str, amt: i64, dur: i64) {
        let mode = match mode.as_bytes().first().map(|b| b.to_ascii_uppercase()) {
            Some(b'Z') => ShakeMode::Zoom,
            Some(b'V') => ShakeMode::Vertical,
            Some(b'H') => ShakeMode::Horizontal,
            _ => ShakeMode::Random,
        };
        let (dx, dy) = match mode {
            ShakeMode::Random => random_shake_direction(amt),
            ShakeMode::Zoom => (0.0, 0.0),
            ShakeMode::Vertical => (0.0, amt as f32),
            ShakeMode::Horizontal => (amt as f32, 0.0),
        };
        self.shake = Some(Shake {
            mode,
            amt,
            ticks_left: dur.max(0),
            dir_t: 0,
            dx,
            dy,
            zoomed: mode == ShakeMode::Zoom,
        });
    }
    pub fn finish_shake(&mut self) {
        self.shake = None;
    }
    pub fn shake_click_skippable(&self) -> bool {
        self.shake.as_ref().is_some_and(|shake| !matches!(shake.mode, ShakeMode::Zoom))
    }
    pub fn start_firefly(&mut self, count: i64, life: i64) {
        self.firefly = Some(Firefly::new(count, life));
    }
    pub fn end_effect(&mut self) {
        self.firefly = None;
    }
    pub fn tick(&mut self) {
        if !self.msg_type_paused {
            if let Some(msg) = &mut self.message {
                msg.type_tick();
            }
        }
        if let Some(tr) = &mut self.active_tr {
            tr.t += 1;
            if tr.t >= tr.spec.frames {
                self.active_tr = None;
                self.prev_bg = None;
            }
        }
        if let Some(tr) = &mut self.panel_tr {
            tr.t = tr.t.saturating_add(1);
            if tr.t >= 256 / tr.speed {
                self.panel_tr = None;
            }
        }
        if let Some(sa) = self.scroll.clone() {
            let next_t = sa.t() + 1;
            let next = match sa {
                ScrollAnim::V { src, target, speed, .. } => {
                    let v = src + speed * next_t as i64 / 10;
                    let reached = if speed <= 0 { v <= target } else { v >= target };
                    if reached {
                        self.set_win(None, Some(target));
                        None
                    } else {
                        self.set_win(None, Some(v));
                        Some(ScrollAnim::V {
                            src,
                            target,
                            speed,
                            t: next_t,
                        })
                    }
                }
                ScrollAnim::H { src, target, speed, .. } => {
                    let v = src + speed * next_t as i64 / 10;
                    let reached = if speed <= 0 { v <= target } else { v >= target };
                    if reached {
                        self.set_win(Some(target), None);
                        None
                    } else {
                        self.set_win(Some(v), None);
                        Some(ScrollAnim::H {
                            src,
                            target,
                            speed,
                            t: next_t,
                        })
                    }
                }
                ScrollAnim::Xy { src_x, src_y, tx, ty, speed, .. } => {
                    let d = speed * next_t as i64 / 10;
                    let dx = (tx - src_x).abs();
                    let dy = (ty - src_y).abs();
                    let major = dx.max(dy);
                    if major == 0 || d >= major {
                        self.set_win(Some(tx), Some(ty));
                        None
                    } else {
                        let sx = if tx < src_x { -1 } else { 1 };
                        let sy = if ty < src_y { -1 } else { 1 };
                        let (nx, ny) = if dx <= dy {
                            (src_x + d * dx / dy * sx, src_y + d * sy)
                        } else {
                            (src_x + d * sx, src_y + d * dy / dx * sy)
                        };
                        self.set_win(Some(nx), Some(ny));
                        Some(ScrollAnim::Xy {
                            src_x,
                            src_y,
                            tx,
                            ty,
                            speed,
                            t: next_t,
                        })
                    }
                }
            };
            self.scroll = next;
        }
        if let Some(sh) = &mut self.shake {
            sh.ticks_left -= 1;
            if sh.ticks_left <= 0 {
                self.shake = None;
            } else {
                sh.dir_t += 1;
                if sh.dir_t >= 3 {
                    sh.dir_t = 0;
                    match sh.mode {
                        ShakeMode::Random => {
                            (sh.dx, sh.dy) = random_shake_direction(sh.amt);
                        }
                        ShakeMode::Vertical => {
                            sh.dy = -sh.dy;
                            if sh.dy == 0.0 {
                                sh.dy = sh.amt as f32;
                            }
                        }
                        ShakeMode::Horizontal => {
                            sh.dx = -sh.dx;
                            if sh.dx == 0.0 {
                                sh.dx = sh.amt as f32;
                            }
                        }
                        ShakeMode::Zoom => {
                            sh.zoomed = !sh.zoomed;
                        }
                    }
                }
            }
        }
        if self.effect_tick {
            if let Some(firefly) = &mut self.firefly {
                firefly.tick();
            }
        }
    }
    fn set_win(&mut self, x: Option<i64>, y: Option<i64>) {
        if let Some(bg) = &mut self.bg {
            if let Some(x) = x {
                bg.win_x = x;
            }
            if let Some(y) = y {
                bg.win_y = y;
            }
        }
    }
    pub fn rebuild_from_system(
        system: &std::collections::BTreeMap<String, String>,
    ) -> Scene {
        let g = |k: &str| system.get(k).cloned().unwrap_or_default();
        let mut s = Scene::default();
        let bg = g("stage_BGFilename");
        if !bg.is_empty() {
            s.stage(
                &bg,
                g("stage_BGX").parse().unwrap_or(0),
                g("stage_BGY").parse().unwrap_or(0),
            );
            s.active_tr = None;
            s.prev_bg = None;
        }
        s.set_transition(
            g("tr_Method").parse().unwrap_or(0),
            None,
            g("tr_Param").parse().unwrap_or(0),
        );
        s.panel_mode = g("panel_Mode").parse().unwrap_or(0);
        s
    }
    pub fn transition_alpha(&self) -> f32 {
        match &self.active_tr {
            None => 1.0,
            Some(tr) if tr.spec.frames == 0 => 1.0,
            Some(tr) => tr.t as f32 / tr.spec.frames as f32,
        }
    }
    pub fn shake_offset(&self) -> (f32, f32) {
        self.shake.as_ref().map(|s| (s.dx, s.dy)).unwrap_or((0.0, 0.0))
    }
    pub fn shake_visual(&self) -> (f32, f32, bool, f32) {
        self.shake
            .as_ref()
            .map(|s| (s.dx, s.dy, s.zoomed, s.amt.max(0) as f32))
            .unwrap_or((0.0, 0.0, false, 0.0))
    }
}
fn rand_u32() -> u32 {
    use std::cell::Cell;
    thread_local! {
        static STATE : Cell < u32 > = Cell::new(1);
    }
    STATE
        .with(|s| {
            let x = s.get().wrapping_mul(214013).wrapping_add(2531011);
            s.set(x);
            (x >> 16) & 0x7fff
        })
}
fn random_shake_direction(amt: i64) -> (f32, f32) {
    let a = amt as f32;
    match rand_u32() % 8 {
        0 => (a, a),
        1 => (a, 0.0),
        2 => (-a, -a),
        3 => (-a, 0.0),
        4 => (a, -a),
        5 => (0.0, -a),
        6 => (-a, a),
        _ => (0.0, a),
    }
}

