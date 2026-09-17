#![cfg(target_arch = "wasm32")]
use eden_engine::audio::{AuxChannel, FadeTicks, MovieAudioStream};
use eden_engine::{AudioBackend, ResourceGroup, ResourceState, ResourceStore};
use js_sys::{Float32Array, Promise, Uint8Array};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_name = edenAudioCreate)]
    fn audio_create() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = edenAudioResume)]
    fn audio_resume(audio: &JsValue) -> Promise;
    #[wasm_bindgen(js_name = edenAudioPlay)]
    fn audio_play(
        audio: &JsValue,
        channel: &str,
        bytes: &Uint8Array,
        name: &str,
        volume: f64,
        pan: f64,
        looping: bool,
        fade_seconds: f64,
    ) -> Promise;
    #[wasm_bindgen(js_name = edenAudioSet)]
    fn audio_set(audio: &JsValue, channel: &str, volume: f64, pan: f64);
    #[wasm_bindgen(js_name = edenAudioStop)]
    fn audio_stop(audio: &JsValue, channel: &str, fade_seconds: f64);
    #[wasm_bindgen(js_name = edenAudioPlaying)]
    fn audio_playing(audio: &JsValue, channel: &str) -> bool;
    #[wasm_bindgen(js_name = edenAudioElapsed)]
    fn audio_elapsed(audio: &JsValue, channel: &str) -> f64;
    #[wasm_bindgen(js_name = edenAudioVoiceLevel)]
    fn audio_voice_level(audio: &JsValue) -> f64;
    #[wasm_bindgen(js_name = edenAudioSpectrum)]
    fn audio_spectrum(audio: &JsValue) -> Float32Array;
}
pub fn create_state() -> Result<JsValue, String> {
    audio_create().map_err(js_error)
}
pub fn resume(state: &JsValue) {
    let _ = audio_resume(state);
}
struct PendingSound {
    group: ResourceGroup,
    candidates: Vec<String>,
    candidate: usize,
    volume: f64,
    pan: f64,
    looping: bool,
    fade_seconds: f64,
}
pub struct WebAudioBackend {
    state: JsValue,
    resources: Rc<dyn ResourceStore>,
    pending: HashMap<&'static str, PendingSound>,
    active_names: HashMap<&'static str, String>,
}
impl WebAudioBackend {
    pub fn new(state: JsValue, resources: Rc<dyn ResourceStore>) -> Self {
        Self {
            state,
            resources,
            pending: HashMap::new(),
            active_names: HashMap::new(),
        }
    }
    fn queue(
        &mut self,
        channel: &'static str,
        group: ResourceGroup,
        name: &str,
        volume: i64,
        pan: i32,
        looping: bool,
        fade_in: FadeTicks,
        muted: bool,
    ) {
        if muted {
            self.stop(channel, None);
            return;
        }
        let volume = percent(volume);
        let pan = (pan as f64 / 100.0).clamp(-1.0, 1.0);
        if (channel == "bgm" || looping)
            && self.active_names.get(channel).is_some_and(|old| old == name)
            && (self.pending.contains_key(channel)
                || audio_playing(&self.state, channel))
        {
            if let Some(pending) = self.pending.get_mut(channel) {
                pending.volume = volume;
                pending.pan = pan;
            }
            audio_set(&self.state, channel, volume, pan);
            return;
        }
        self.stop(channel, None);
        let mut candidates = vec![name.to_ascii_lowercase()];
        if !name.contains('.') {
            candidates.push(format!("{}.ogg", name.to_ascii_lowercase()));
        }
        self.active_names.insert(channel, name.to_owned());
        self.pending
            .insert(
                channel,
                PendingSound {
                    group,
                    candidates,
                    candidate: 0,
                    volume,
                    pan,
                    looping,
                    fade_seconds: fade_in.unwrap_or(0).max(0) as f64 / 100.0,
                },
            );
    }
    fn stop(&mut self, channel: &'static str, fade: FadeTicks) {
        self.pending.remove(channel);
        self.active_names.remove(channel);
        let seconds = fade.unwrap_or(0).max(0) as f64;
        audio_stop(&self.state, channel, seconds);
    }
    fn process_pending(&mut self) {
        const CHANNELS: [&str; 7] = [
            "bgm",
            "se1",
            "se2",
            "voice",
            "aux-ui",
            "aux-bgm",
            "aux-voice",
        ];
        for channel in CHANNELS {
            let Some(pending) = self.pending.get(channel) else {
                continue;
            };
            let name = pending.candidates[pending.candidate].clone();
            let state = self.resources.request_bytes(pending.group, &name);
            match state {
                ResourceState::Pending => {}
                ResourceState::Ready(bytes) => {
                    let bytes = Uint8Array::from(bytes.as_ref());
                    let pending = self.pending.remove(channel).unwrap();
                    let _ = audio_play(
                        &self.state,
                        channel,
                        &bytes,
                        &name,
                        pending.volume,
                        pending.pan,
                        pending.looping,
                        pending.fade_seconds,
                    );
                }
                ResourceState::Missing => {
                    let pending = self.pending.get_mut(channel).unwrap();
                    pending.candidate += 1;
                    if pending.candidate >= pending.candidates.len() {
                        log::warn!(
                            "WebAudio 资源缺失 {}:{name}", pending.group.name()
                        );
                        self.pending.remove(channel);
                        self.active_names.remove(channel);
                    }
                }
                ResourceState::Failed(error) => {
                    log::error!(
                        "WebAudio 资源读取失败 {}:{name}: {error}", pending.group
                        .name()
                    );
                    self.pending.remove(channel);
                    self.active_names.remove(channel);
                }
            }
        }
    }
}
impl AudioBackend for WebAudioBackend {
    fn tick(&mut self) {
        self.process_pending();
    }
    fn play_bgm(
        &mut self,
        name: &str,
        volume: i64,
        pan: i32,
        looping: bool,
        fade_in: FadeTicks,
        _fade_out: FadeTicks,
        muted: bool,
    ) {
        self.queue(
            "bgm",
            ResourceGroup::Bgm,
            name,
            volume,
            pan,
            looping,
            fade_in,
            muted,
        );
    }
    fn stop_bgm(&mut self, fade: FadeTicks) {
        self.stop("bgm", fade);
    }
    fn play_se(
        &mut self,
        name: &str,
        volume: i64,
        pan: i32,
        se2: bool,
        looping: bool,
        fade_in: FadeTicks,
        _fade_out: FadeTicks,
        muted: bool,
    ) {
        self.queue(
            if se2 { "se2" } else { "se1" },
            ResourceGroup::Se,
            name,
            volume,
            pan,
            looping,
            fade_in,
            muted,
        );
    }
    fn stop_se(&mut self, se2: bool, fade: FadeTicks) {
        self.stop(if se2 { "se2" } else { "se1" }, fade);
    }
    fn play_voice(&mut self, name: &str, volume: i64, pan: i32) {
        self.queue("voice", ResourceGroup::Voice, name, volume, pan, false, None, false);
    }
    fn stop_voice(&mut self) {
        self.stop("voice", None);
    }
    fn play_aux(&mut self, name: &str, volume: i64, channel: AuxChannel) {
        let channel = match channel {
            AuxChannel::Ui | AuxChannel::TestSe => "aux-ui",
            AuxChannel::TestBgm => "aux-bgm",
            AuxChannel::TestVoice => "aux-voice",
        };
        self.queue(channel, ResourceGroup::Sys, name, volume, 0, false, None, false);
    }
    fn stop_all(&mut self) {
        for channel in ["bgm", "se1", "se2", "voice", "aux-ui", "aux-bgm", "aux-voice"] {
            self.stop(channel, None);
        }
    }
    fn start_movie_audio(&mut self, _stream: MovieAudioStream, _volume: i64) -> bool {
        false
    }
    fn stop_movie_audio(&mut self) {}
    fn bgm_elapsed_secs(&self) -> Option<f32> {
        let elapsed = audio_elapsed(&self.state, "bgm");
        (elapsed >= 0.0).then_some(elapsed as f32)
    }
    fn bgm_playing(&self) -> bool {
        self.pending.contains_key("bgm") || audio_playing(&self.state, "bgm")
    }
    fn voice_playing(&self) -> bool {
        self.pending.contains_key("voice") || audio_playing(&self.state, "voice")
    }
    fn voice_level(&self) -> f32 {
        audio_voice_level(&self.state).clamp(0.0, 1.0) as f32
    }
    fn spectrum(&self, out: &mut [f32]) {
        let values = audio_spectrum(&self.state);
        let count = out.len().min(values.length() as usize);
        values.subarray(0, count as u32).copy_to(&mut out[..count]);
        out[count..].fill(0.0);
    }
}
fn percent(value: i64) -> f64 {
    (value.clamp(0, 100) as f64) / 100.0
}
fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
        .unwrap_or_else(|| "未知 WebAudio 错误".into())
}
