use crate::cgdata::MUSIC_TRACKS;
pub const MODE_TRACKS: usize = 16;
pub const MODES: usize = 3;
pub const TRACK_HIT: [f32; 4] = [110.0, 34.0, 584.0, 546.0];
pub const ROW_H: f32 = 32.0;
pub const BTN_PREV: [f32; 4] = [688.0, 160.0, 736.0, 208.0];
pub const BTN_PLAY: [f32; 4] = [752.0, 160.0, 800.0, 208.0];
pub const BTN_STOP: [f32; 4] = [816.0, 160.0, 864.0, 208.0];
pub const BTN_NEXT: [f32; 4] = [880.0, 160.0, 928.0, 208.0];
pub const BTN_MODE: [f32; 4] = [96.0, 570.0, 448.0, 612.0];
pub const BTN_PREVPAGE: [f32; 4] = [600.0, 570.0, 720.0, 612.0];
pub const BTN_NEXTPAGE: [f32; 4] = [720.0, 570.0, 844.0, 612.0];
pub const BTN_EXIT: [f32; 4] = [844.0, 570.0, 946.0, 612.0];
pub const TIME_MIN_AT: [f32; 2] = [769.0, 81.0];
pub const TIME_SEC_AT: [f32; 2] = [865.0, 81.0];
pub const TRACK_NO_AT: [f32; 2] = [896.0, 39.0];
pub const METER_RECT: [f32; 4] = [760.0, 270.0, 920.0, 345.0];
pub const METER_BARS: usize = 20;
pub const APPEND_INSTALLED: bool = false;
pub fn track_name(abs: usize) -> Option<&'static str> {
    let n = MUSIC_TRACKS.get(abs)?;
    (!n.is_empty()).then_some(n)
}
pub struct MusicMenu {
    pub mode: usize,
    pub selected: i32,
    pub playing: bool,
    pub playing_abs: i32,
    pub highlight: i32,
    pub elapsed: u32,
    pub spectrum: Vec<f32>,
    pub last_click: Option<(i32, web_time::Instant)>,
    pub fade: u8,
    closing: bool,
    pub dead: bool,
}
impl MusicMenu {
    pub fn open() -> Self {
        MusicMenu {
            mode: 0,
            selected: -1,
            playing: false,
            playing_abs: -1,
            highlight: -1,
            elapsed: 0,
            spectrum: vec![0.0; METER_BARS],
            last_click: None,
            fade: 0,
            closing: false,
            dead: false,
        }
    }
    pub fn base(&self) -> String {
        format!("music{}.png", self.mode + 1)
    }
    pub fn row_track(&self, row: i32) -> Option<usize> {
        if !(0..MODE_TRACKS as i32).contains(&row) {
            return None;
        }
        let abs = self.mode * MODE_TRACKS + row as usize;
        track_name(abs).map(|_| row as usize)
    }
    pub fn play(&mut self, row: i32) -> Option<&'static str> {
        let row = self.row_track(row)?;
        let abs = self.mode * MODE_TRACKS + row;
        let name = track_name(abs)?;
        self.playing_abs = abs as i32;
        self.playing = true;
        self.elapsed = 0;
        Some(name)
    }
    pub fn stop(&mut self) {
        self.playing = false;
        self.playing_abs = -1;
        self.highlight = -1;
    }
    pub fn advance(&mut self) -> Option<&'static str> {
        let mut idx = (self.playing_abs + 1).max(0) as usize;
        if idx / MODE_TRACKS == 2 && !APPEND_INSTALLED || !track_exists(idx) {
            idx = 0;
        }
        if !track_exists(idx) {
            self.stop();
            return None;
        }
        self.playing_abs = idx as i32;
        self.elapsed = 0;
        track_name(idx)
    }
    pub fn step(&mut self, dir: i32) -> Option<&'static str> {
        let cur = if self.selected >= 0 { self.selected } else { 0 };
        let abs_cur = self.mode * MODE_TRACKS + cur as usize;
        let mut abs_next = (abs_cur as i64 + dir as i64).rem_euclid(48) as usize;
        if !track_exists(abs_next) {
            abs_next = 0;
        }
        let (m, t) = (abs_next / MODE_TRACKS, (abs_next % MODE_TRACKS) as i32);
        if m != self.mode {
            self.mode = m;
            self.selected = -1;
        }
        self.selected = t;
        if self.playing {
            let name = track_name(abs_next)?;
            self.playing_abs = abs_next as i32;
            self.elapsed = 0;
            Some(name)
        } else {
            None
        }
    }
    pub fn toggle_mode(&mut self) -> bool {
        if !APPEND_INSTALLED {
            return false;
        }
        self.mode = if self.mode == 2 { 0 } else { 2 };
        self.selected = 0;
        self.mode_changed()
    }
    pub fn turn_mode(&mut self, dir: i32) -> bool {
        let next = if dir < 0 {
            if self.mode == 0 {
                if APPEND_INSTALLED { 2 } else { 1 }
            } else {
                self.mode - 1
            }
        } else {
            match self.mode {
                0 => 1,
                1 if APPEND_INSTALLED => 2,
                _ => 0,
            }
        };
        self.mode = next;
        self.selected = 0;
        self.mode_changed()
    }
    fn mode_changed(&mut self) -> bool {
        self.playing = false;
        self.playing_abs = -1;
        self.highlight = 0;
        true
    }
    pub fn close(&mut self) {
        self.closing = true;
    }
    pub fn tick(&mut self) {
        if self.closing {
            self.fade = self.fade.saturating_sub(10);
            if self.fade == 0 {
                self.dead = true;
            }
        } else {
            self.fade = (self.fade as u16 + 10).min(255) as u8;
        }
        if self.playing_abs >= 0 {
            let abs = self.playing_abs as usize;
            if abs / MODE_TRACKS == self.mode {
                self.highlight = (abs % MODE_TRACKS) as i32;
            }
        }
    }
    pub fn alpha(&self) -> f32 {
        self.fade as f32 / 255.0
    }
}
fn track_exists(abs: usize) -> bool {
    track_name(abs).is_some()
}

