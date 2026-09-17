use crate::settings::Settings;
pub const WIDGETS: &[(f32, f32, f32, f32, u8)] = &[
    (133.0, 100.0, 357.0, 136.0, 1),
    (133.0, 176.0, 357.0, 212.0, 2),
    (133.0, 248.0, 357.0, 292.0, 3),
    (341.0, 440.0, 369.0, 468.0, 4),
    (341.0, 468.0, 369.0, 496.0, 5),
    (133.0, 532.0, 233.0, 566.0, 6),
    (133.0, 566.0, 233.0, 600.0, 7),
    (405.0, 64.0, 621.0, 108.0, 8),
    (405.0, 108.0, 621.0, 148.0, 9),
    (405.0, 192.0, 621.0, 236.0, 10),
    (405.0, 236.0, 621.0, 280.0, 11),
    (405.0, 280.0, 621.0, 320.0, 12),
    (405.0, 320.0, 621.0, 360.0, 13),
    (405.0, 408.0, 621.0, 459.0, 14),
    (405.0, 464.0, 621.0, 508.0, 15),
    (405.0, 556.0, 621.0, 600.0, 16),
    (669.0, 96.0, 889.0, 136.0, 17),
    (669.0, 172.0, 889.0, 212.0, 18),
    (669.0, 248.0, 889.0, 288.0, 19),
    (833.0, 70.0, 877.0, 92.0, 23),
    (833.0, 146.0, 877.0, 168.0, 24),
    (833.0, 218.0, 877.0, 240.0, 25),
    (777.0, 70.0, 833.0, 92.0, 20),
    (777.0, 146.0, 833.0, 168.0, 21),
    (777.0, 218.0, 833.0, 240.0, 22),
    (665.0, 368.0, 785.0, 408.0, 26),
    (785.0, 368.0, 877.0, 408.0, 27),
    (665.0, 408.0, 785.0, 444.0, 28),
    (785.0, 408.0, 877.0, 444.0, 29),
    (665.0, 444.0, 785.0, 480.0, 30),
    (785.0, 444.0, 877.0, 480.0, 31),
    (665.0, 484.0, 785.0, 540.0, 32),
    (785.0, 484.0, 877.0, 540.0, 33),
    (681.0, 543.0, 757.0, 595.0, 100),
    (776.0, 543.0, 904.0, 595.0, 101),
];
pub const SLIDERS: &[(u8, f32, f32, f32)] = &[
    (1, 133.0, 138.0, 112.0),
    (2, 133.0, 138.0, 187.0),
    (3, 133.0, 138.0, 262.0),
    (17, 669.0, 674.0, 108.0),
    (18, 669.0, 674.0, 183.0),
    (19, 669.0, 674.0, 258.0),
];
pub const MUTE_CHECKS: [(f32, f32); 3] = [(781.0, 62.0), (781.0, 138.0), (781.0, 210.0)];
pub const VOICE_CHECKS: [(f32, f32); 8] = [
    (671.0, 374.0),
    (792.0, 374.0),
    (671.0, 411.0),
    (792.0, 411.0),
    (671.0, 448.0),
    (792.0, 448.0),
    (671.0, 485.0),
    (792.0, 485.0),
];
pub const CHECKS: &[(u8, f32, f32)] = &[
    (6, 136.0, 534.0),
    (7, 136.0, 568.0),
    (10, 415.0, 197.0),
    (11, 415.0, 240.0),
    (12, 415.0, 284.0),
    (13, 415.0, 324.0),
    (14, 415.0, 413.0),
    (15, 415.0, 468.0),
    (16, 415.0, 561.0),
];
pub fn check_state(s: &Settings, cmd: u8) -> bool {
    match cmd {
        6 => s.play_mode != 0,
        7 => s.play_mode == 0,
        10 => s.skip_showing != 0,
        11 => s.screen_effect != 0,
        12 => s.shadow != 0,
        13 => s.animation != 0,
        14 => s.backlog_voice_playback != 0,
        15 => s.stop_voice_at_next != 0,
        16 => s.progress_in_background != 0,
        _ => false,
    }
}
pub fn hit(x: f32, y: f32) -> Option<u8> {
    WIDGETS
        .iter()
        .find(|&&(x0, y0, x1, y1, _)| x >= x0 && x < x1 && y >= y0 && y < y1)
        .map(|&(_, _, _, _, cmd)| cmd)
}
pub fn slider_value(cmd: u8, x: f32) -> i64 {
    let x0 = SLIDERS.iter().find(|s| s.0 == cmd).map(|s| s.1).unwrap_or(0.0);
    let v = (x - x0 - 11.0).clamp(0.0, 200.0) as i64;
    v * 100 / 200
}
pub struct ConfigState {
    fade_in: u32,
    fade_out: u32,
    closing: bool,
    pub dead: bool,
    pub drag: Option<u8>,
    pub hover: Option<u8>,
    pub pressed_button: Option<u8>,
    pub screen_mode: i64,
    pub font_name: String,
    pub original_font_name: String,
}
impl ConfigState {
    pub fn new(s: &Settings, active_font: &str) -> Self {
        ConfigState {
            fade_in: 0,
            fade_out: 0,
            closing: false,
            dead: false,
            drag: None,
            hover: None,
            pressed_button: None,
            screen_mode: s.screen_mode,
            font_name: active_font.to_string(),
            original_font_name: active_font.to_string(),
        }
    }
    pub fn is_active(&self) -> bool {
        !self.closing && self.fade_in >= 255
    }
    pub fn begin_close(&mut self) {
        self.closing = true;
        self.drag = None;
        self.pressed_button = None;
    }
    pub fn tick(&mut self) {
        if self.closing {
            self.fade_out = (self.fade_out + 4).min(255);
            if self.fade_out >= 255 {
                self.dead = true;
            }
        } else {
            self.fade_in = (self.fade_in + 4).min(255);
        }
    }
    pub fn alpha(&self) -> f32 {
        let a = self.fade_in.saturating_sub(self.fade_out).min(255);
        a as f32 / 255.0
    }
}

