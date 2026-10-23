#[cfg(not(target_arch = "wasm32"))]
pub mod cpal_backend;
pub mod mixer;
pub use mixer::MovieAudioStream;
pub type FadeTicks = Option<i64>;
#[derive(Clone, Copy, Debug)]
pub enum AuxChannel {
    Ui,
    TestBgm,
    TestVoice,
    TestSe,
}
pub fn parse_vol_pan(name: &str, base: i64) -> (String, i64, i32) {
    let (vol, pan) = (base, 0i32);
    let Some(lb) = name.rfind('[') else {
        return (name.to_string(), vol, pan);
    };
    let stripped = name[..lb].to_string();
    let Some(rel) = name[lb + 1..].find(']') else {
        return (stripped, vol, pan);
    };
    let inner = &name[lb + 1..lb + 1 + rel];
    let (v_str, p_str) = match inner.split_once(',') {
        Some((v, p)) => (v, Some(p)),
        None => (inner, None),
    };
    let v = scan_int_prefix(v_str).unwrap_or(100).clamp(0, 100);
    let p = p_str.and_then(scan_int_prefix).unwrap_or(0).clamp(-100, 100);
    (stripped, base * v / 100, p as i32)
}
fn scan_int_prefix(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    s[..i].parse().ok()
}
pub trait AudioBackend {
    fn tick(&mut self) {}
    fn set_paused(&mut self, _paused: bool) -> anyhow::Result<()> { Ok(()) }
    fn play_bgm(
        &mut self,
        name: &str,
        volume: i64,
        pan: i32,
        looping: bool,
        fade_in: FadeTicks,
        fade_out: FadeTicks,
        muted: bool,
    );
    fn stop_bgm(&mut self, fade: FadeTicks);
    fn play_se(
        &mut self,
        name: &str,
        volume: i64,
        pan: i32,
        se2: bool,
        looping: bool,
        fade_in: FadeTicks,
        fade_out: FadeTicks,
        muted: bool,
    );
    fn stop_se(&mut self, se2: bool, fade: FadeTicks);
    fn play_voice(&mut self, name: &str, volume: i64, pan: i32);
    fn stop_voice(&mut self);
    fn play_aux(&mut self, name: &str, volume: i64, channel: AuxChannel);
    fn stop_all(&mut self);
    fn start_movie_audio(&mut self, stream: MovieAudioStream, volume: i64) -> bool;
    fn stop_movie_audio(&mut self);
    fn bgm_elapsed_secs(&self) -> Option<f32>;
    fn bgm_playing(&self) -> bool;
    fn voice_playing(&self) -> bool;
    fn voice_level(&self) -> f32;
    fn spectrum(&self, out: &mut [f32]);
}
#[derive(Default)]
pub struct NullAudio {
    pub last_bgm: Option<String>,
    pub last_se: [Option<String>; 2],
    pub last_voice: Option<String>,
}
impl AudioBackend for NullAudio {
    fn play_bgm(
        &mut self,
        name: &str,
        _volume: i64,
        _pan: i32,
        _looping: bool,
        _fade_in: FadeTicks,
        _fade_out: FadeTicks,
        _muted: bool,
    ) {
        log::debug!("bgm(muted): {name}");
        self.last_bgm = Some(name.to_string());
    }
    fn stop_bgm(&mut self, _fade: FadeTicks) {
        self.last_bgm = None;
    }
    fn play_se(
        &mut self,
        name: &str,
        _volume: i64,
        _pan: i32,
        se2: bool,
        _looping: bool,
        _fade_in: FadeTicks,
        _fade_out: FadeTicks,
        _muted: bool,
    ) {
        log::debug!("se{}(muted): {name}", if se2 { 2 } else { 1 });
        self.last_se[usize::from(se2)] = Some(name.to_string());
    }
    fn stop_se(&mut self, se2: bool, _fade: FadeTicks) {
        self.last_se[usize::from(se2)] = None;
    }
    fn play_voice(&mut self, name: &str, _volume: i64, _pan: i32) {
        log::debug!("voice(muted): {name}");
        self.last_voice = Some(name.to_string());
    }
    fn stop_voice(&mut self) {
        self.last_voice = None;
    }
    fn play_aux(&mut self, name: &str, _volume: i64, _channel: AuxChannel) {
        log::debug!("aux(muted): {name}");
    }
    fn stop_all(&mut self) {
        self.last_bgm = None;
        self.last_se = [None, None];
        self.last_voice = None;
    }
    fn start_movie_audio(&mut self, stream: MovieAudioStream, _volume: i64) -> bool {
        stream.discard();
        false
    }
    fn stop_movie_audio(&mut self) {}
    fn bgm_elapsed_secs(&self) -> Option<f32> {
        None
    }
    fn bgm_playing(&self) -> bool {
        false
    }
    fn voice_playing(&self) -> bool {
        false
    }
    fn voice_level(&self) -> f32 {
        0.0
    }
    fn spectrum(&self, out: &mut [f32]) {
        out.fill(0.0);
    }
}

