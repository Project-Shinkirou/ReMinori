use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub message_speed_tbr: i64,
    pub message_speed_read: i64,
    pub message_speed_autoplay: i64,
    pub fontname: String,
    pub play_mode: i64,
    pub screen_mode: i64,
    pub screen_effect: i64,
    pub animation: i64,
    pub shadow: i64,
    pub skip_showing: i64,
    pub backlog_voice_playback: i64,
    pub stop_voice_at_next: i64,
    pub progress_in_background: i64,
    pub volume_bgm: i64,
    pub volume_voice: i64,
    pub volume_se: i64,
    pub full_screen_mode: i64,
    pub set_window_priority: i64,
    pub mute_bgm: bool,
    pub mute_voice: bool,
    pub mute_se: bool,
    pub quick_save_file_number: i64,
    pub last_save_page_number: i64,
    pub auto_save_cycle: i64,
    pub auto_save_backlog_limit: i64,
    pub bg_cache_count: i64,
    pub st_cache_count: i64,
    pub effect_draw_interval: i64,
    pub transition_draw_interval: i64,
    pub scroll_draw_interval: i64,
    pub shake_draw_interval: i64,
    pub joystick_usage: i64,
    pub blt_once: i64,
    pub cursor_control: i64,
    pub cls: i64,
    pub stretch_acc: i64,
    pub stretch_aa: i64,
    pub voice_chars: [bool; 8],
    pub window_x: i64,
    pub window_y: i64,
    pub window_width: i64,
    pub window_height: i64,
    pub window_mode: i64,
}
pub const VOICE_KEYS: [&str; 8] = [
    "sio",
    "ryo",
    "eri",
    "ina",
    "rav",
    "may",
    "nat",
    "etc",
];
impl Default for Settings {
    fn default() -> Self {
        Settings {
            message_speed_tbr: 50,
            message_speed_read: 50,
            message_speed_autoplay: 50,
            fontname: "ＭＳ Ｐゴシック".into(),
            play_mode: 1,
            screen_mode: 0,
            screen_effect: 1,
            animation: 1,
            shadow: 1,
            skip_showing: 1,
            backlog_voice_playback: 1,
            stop_voice_at_next: 0,
            progress_in_background: 0,
            volume_bgm: 100,
            volume_voice: 100,
            volume_se: 100,
            full_screen_mode: -1,
            set_window_priority: 0,
            mute_bgm: false,
            mute_voice: false,
            mute_se: false,
            quick_save_file_number: 1,
            last_save_page_number: 0,
            auto_save_cycle: 200,
            auto_save_backlog_limit: 100,
            bg_cache_count: 3,
            st_cache_count: 10,
            effect_draw_interval: 10,
            transition_draw_interval: 5,
            scroll_draw_interval: 2,
            shake_draw_interval: 3,
            joystick_usage: 1,
            blt_once: 1,
            cursor_control: 0,
            cls: 0,
            stretch_acc: 1,
            stretch_aa: 1,
            voice_chars: [true; 8],
            window_x: 0,
            window_y: 0,
            window_width: 0,
            window_height: 0,
            window_mode: 0,
        }
    }
}
pub fn settings_path() -> PathBuf {
    crate::engine::Engine::save_dir().join("settings.tsv")
}
impl Settings {
    pub fn load_or_default() -> Settings {
        Settings::load(&settings_path())
    }
    pub fn persist(&self) {
        self.save(&settings_path());
    }
    pub fn load(path: &std::path::Path) -> Settings {
        std::fs::read(path)
            .ok()
            .map_or_else(Settings::default, |bytes| Settings::from_bytes(&bytes))
    }
    pub fn from_bytes(bytes: &[u8]) -> Settings {
        let mut s = Settings::default();
        let text = String::from_utf8_lossy(bytes);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            let n = || v.parse::<i64>().unwrap_or(0);
            match k {
                "messageSpeedTBR" => s.message_speed_tbr = n(),
                "messageSpeedRead" => s.message_speed_read = n(),
                "messageSpeedAutoPlay" => s.message_speed_autoplay = n(),
                "fontname" => s.fontname = v.to_string(),
                "playMode" => s.play_mode = n(),
                "screenMode" => s.screen_mode = n(),
                "screenEffect" => s.screen_effect = n(),
                "animation" => s.animation = n(),
                "shadow" => s.shadow = n(),
                "skipShowing" => s.skip_showing = n(),
                "backlogVoicePlayback" => s.backlog_voice_playback = n(),
                "stopVoiceAtNext" => s.stop_voice_at_next = n(),
                "progressInBackground" => s.progress_in_background = n(),
                "volumeBGM" => s.volume_bgm = n(),
                "volumeVoice" => s.volume_voice = n(),
                "volumeSE" => s.volume_se = n(),
                "fullScreenMode" => s.full_screen_mode = n(),
                "setWindowPriority" => s.set_window_priority = n(),
                "muteBGM" => s.mute_bgm = n() != 0,
                "muteVoice" => s.mute_voice = n() != 0,
                "muteSE" => s.mute_se = n() != 0,
                "quickSaveFileNumber" => s.quick_save_file_number = n(),
                "lastSavePageNumber" => s.last_save_page_number = n(),
                "autoSaveCycle" => s.auto_save_cycle = n(),
                "autoSaveBacklogLimit" => s.auto_save_backlog_limit = n(),
                "bgCacheCount" => s.bg_cache_count = n(),
                "stCacheCount" => s.st_cache_count = n(),
                "effectDrawInterval" => s.effect_draw_interval = n(),
                "transitionDrawInterval" => s.transition_draw_interval = n(),
                "scrollDrawInterval" => s.scroll_draw_interval = n(),
                "shakeDrawInterval" => s.shake_draw_interval = n(),
                "joystickUsage" => s.joystick_usage = n(),
                "bltOnce" => s.blt_once = n(),
                "cursorControl" => s.cursor_control = n(),
                "cls" => s.cls = n(),
                "stretchAcc" => s.stretch_acc = n(),
                "stretchAA" => s.stretch_aa = n(),
                "windowXpos" => s.window_x = n(),
                "windowYpos" => s.window_y = n(),
                "windowWidth" => s.window_width = n(),
                "windowHeight" => s.window_height = n(),
                "windowMode" => s.window_mode = n(),
                _ => {
                    if let Some(i) = VOICE_KEYS.iter().position(|key| *key == k) {
                        s.voice_chars[i] = n() != 0;
                    }
                }
            }
        }
        s
    }
    pub fn save(&self, path: &std::path::Path) {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
        let _ = std::fs::write(path, self.to_bytes());
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        fn line(out: &mut String, k: &str, v: i64) {
            out.push_str(k);
            out.push('=');
            out.push_str(&v.to_string());
            out.push('\n');
        }
        let mut out = String::new();
        line(&mut out, "messageSpeedTBR", self.message_speed_tbr);
        line(&mut out, "messageSpeedRead", self.message_speed_read);
        line(&mut out, "messageSpeedAutoPlay", self.message_speed_autoplay);
        if !self.fontname.is_empty() {
            out.push_str("fontname=");
            out.push_str(&self.fontname);
            out.push('\n');
        }
        line(&mut out, "playMode", self.play_mode);
        line(&mut out, "screenMode", self.screen_mode);
        line(&mut out, "screenEffect", self.screen_effect);
        line(&mut out, "animation", self.animation);
        line(&mut out, "shadow", self.shadow);
        line(&mut out, "skipShowing", self.skip_showing);
        line(&mut out, "backlogVoicePlayback", self.backlog_voice_playback);
        line(&mut out, "stopVoiceAtNext", self.stop_voice_at_next);
        line(&mut out, "progressInBackground", self.progress_in_background);
        line(&mut out, "volumeBGM", self.volume_bgm);
        line(&mut out, "volumeVoice", self.volume_voice);
        line(&mut out, "volumeSE", self.volume_se);
        line(&mut out, "fullScreenMode", self.full_screen_mode);
        line(&mut out, "setWindowPriority", self.set_window_priority);
        line(&mut out, "muteBGM", self.mute_bgm as i64);
        line(&mut out, "muteVoice", self.mute_voice as i64);
        line(&mut out, "muteSE", self.mute_se as i64);
        line(&mut out, "quickSaveFileNumber", self.quick_save_file_number);
        line(&mut out, "lastSavePageNumber", self.last_save_page_number);
        line(&mut out, "autoSaveCycle", self.auto_save_cycle);
        line(&mut out, "autoSaveBacklogLimit", self.auto_save_backlog_limit);
        line(&mut out, "bgCacheCount", self.bg_cache_count);
        line(&mut out, "stCacheCount", self.st_cache_count);
        line(&mut out, "effectDrawInterval", self.effect_draw_interval);
        line(&mut out, "transitionDrawInterval", self.transition_draw_interval);
        line(&mut out, "scrollDrawInterval", self.scroll_draw_interval);
        line(&mut out, "shakeDrawInterval", self.shake_draw_interval);
        line(&mut out, "joystickUsage", self.joystick_usage);
        line(&mut out, "bltOnce", self.blt_once);
        line(&mut out, "cursorControl", self.cursor_control);
        line(&mut out, "cls", self.cls);
        line(&mut out, "stretchAcc", self.stretch_acc);
        line(&mut out, "stretchAA", self.stretch_aa);
        line(&mut out, "windowXpos", self.window_x);
        line(&mut out, "windowYpos", self.window_y);
        line(&mut out, "windowWidth", self.window_width);
        line(&mut out, "windowHeight", self.window_height);
        line(&mut out, "windowMode", self.window_mode);
        for (i, k) in VOICE_KEYS.iter().enumerate() {
            line(&mut out, k, self.voice_chars[i] as i64);
        }
        out.into_bytes()
    }
    pub fn voice_flag(&self, key: &str) -> bool {
        VOICE_KEYS.iter().position(|k| *k == key).map_or(true, |i| self.voice_chars[i])
    }
    pub fn window_rect(&self) -> Option<(i32, i32, u32, u32)> {
        if self.window_width <= 0 || self.window_height <= 0 {
            return None;
        }
        Some((
            self.window_x as i32,
            self.window_y as i32,
            self.window_width as u32,
            self.window_height as u32,
        ))
    }
    pub fn window_maximized(&self) -> bool {
        self.window_mode == 3
    }
}

