use super::mixer::{
    self, LoaderCmd, MixerShared, ResampBank, SlotKind, AUX_RESERVED, AUX_TEST_BGM,
    AUX_TEST_SE, AUX_TEST_VOICE, AUX_UI, SLOTS,
};
use super::{AudioBackend, AuxChannel, FadeTicks, MovieAudioStream};
use crate::vfs::Vfs;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;
const ACTIVE_BGM: usize = 0;
const ACTIVE_VOICE: usize = 1;
const ACTIVE_SE1: usize = 2;
const ACTIVE_SE2: usize = 3;
pub struct CpalAudio {
    shared: Arc<MixerShared>,
    tx: Sender<LoaderCmd>,
    _stream: cpal::Stream,
    vfs: Rc<Vfs>,
    names: RefCell<[Option<String>; SLOTS]>,
    active: RefCell<[Option<usize>; 4]>,
    cursor: usize,
}
impl CpalAudio {
    pub fn new(vfs: Rc<Vfs>) -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device
            .supported_output_configs()
            .ok()?
            .find(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
            .or_else(|| {
                device.supported_output_configs().ok()?.find(|c| c.channels() == 2)
            })?;
        let rate = supported.min_sample_rate().max(cpal::SampleRate(44100)).0;
        let sample_format = supported.sample_format();
        let config = supported.with_sample_rate(cpal::SampleRate(rate)).config();
        let shared = Arc::new(MixerShared::new(config.sample_rate.0));
        let (tx, rx) = std::sync::mpsc::channel::<LoaderCmd>();
        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("eden-audio-loader".into())
                .spawn(move || mixer::loader_loop(rx, shared))
                .ok()?;
        }
        let err = |e| log::warn!("音频设备错误: {e}");
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut bank = ResampBank::new();
                let sh = Arc::clone(&shared);
                device
                    .build_output_stream(
                        &config,
                        move |data: &mut [f32], _| sh.mix(&mut bank, data),
                        err,
                        None,
                    )
                    .ok()?
            }
            cpal::SampleFormat::I16 => {
                let mut bank = ResampBank::new();
                let sh = Arc::clone(&shared);
                device
                    .build_output_stream(
                        &config,
                        move |data: &mut [i16], _| {
                            let mut tmp = vec![0f32; data.len()];
                            sh.mix(&mut bank, &mut tmp);
                            for (d, s) in data.iter_mut().zip(tmp) {
                                *d = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                            }
                        },
                        err,
                        None,
                    )
                    .ok()?
            }
            fmt => {
                log::warn!("不支持的输出采样格式 {fmt:?}");
                return None;
            }
        };
        stream.play().ok()?;
        log::info!(
            "音频输出: default @ {}Hz {:?}", config.sample_rate.0, sample_format
        );
        Some(CpalAudio {
            shared,
            tx,
            _stream: stream,
            vfs,
            names: RefCell::new(std::array::from_fn(|_| None)),
            active: RefCell::new([None; 4]),
            cursor: mixer::STREAM_SLOTS - 1,
        })
    }
    fn fetch(&self, group: &'static str, name: &str) -> Option<Vec<u8>> {
        let key = name.to_ascii_lowercase();
        self.vfs
            .read_entry_debug(group, &key)
            .or_else(|| {
                if group == "sys" || key.contains('.') {
                    None
                } else {
                    self.vfs.read_entry_debug(group, &format!("{key}.ogg"))
                }
            })
    }
    fn active_slot(&self, channel: usize) -> Option<usize> {
        self.active.borrow()[channel]
    }
    fn same_active_name(&self, channel: usize, name: &str) -> Option<usize> {
        let slot = self.active_slot(channel)?;
        (self.shared.slot_alive(slot)
            && self.names.borrow()[slot].as_deref() == Some(name))
            .then_some(slot)
    }
    fn stop_active(&mut self, channel: usize, fade: FadeTicks) {
        let Some(slot) = self.active.borrow_mut()[channel].take() else {
            return;
        };
        self.shared.stop_fade(&self.tx, slot, fade);
    }
    fn start_stream(
        &mut self,
        group: &'static str,
        name: &str,
        volume: i64,
        pan: i32,
        looping: bool,
        fade_in: FadeTicks,
        kind: SlotKind,
    ) -> Option<usize> {
        let Some(bytes) = self.fetch(group, name) else {
            log::warn!("音频资源缺失 ({group}): {name}");
            return None;
        };
        let Some(slot) = self.shared.alloc_stream(&mut self.cursor) else {
            log::warn!("音频动态流槽已满: {name}");
            return None;
        };
        self.shared.start(&self.tx, slot, bytes, looping, volume, pan, fade_in, kind);
        self.names.borrow_mut()[slot] = Some(name.to_string());
        Some(slot)
    }
}
impl AudioBackend for CpalAudio {
    fn play_bgm(
        &mut self,
        name: &str,
        volume: i64,
        pan: i32,
        looping: bool,
        fade_in: FadeTicks,
        fade_out: FadeTicks,
        muted: bool,
    ) {
        if let Some(slot) = self.same_active_name(ACTIVE_BGM, name) {
            self.shared.fade_volume(slot, volume, fade_in);
            self.shared.set_pan(slot, pan);
            return;
        }
        self.stop_active(ACTIVE_BGM, fade_out);
        if muted {
            return;
        }
        if let Some(slot) = self
            .start_stream("bgm", name, volume, pan, looping, fade_in, SlotKind::Bgm)
        {
            self.active.borrow_mut()[ACTIVE_BGM] = Some(slot);
        }
    }
    fn stop_bgm(&mut self, fade: FadeTicks) {
        self.stop_active(ACTIVE_BGM, fade);
    }
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
    ) {
        let channel = if se2 { ACTIVE_SE2 } else { ACTIVE_SE1 };
        if looping {
            if let Some(slot) = self.same_active_name(channel, name) {
                self.shared.set_volume(slot, volume);
                self.shared.set_pan(slot, pan);
                return;
            }
        }
        self.stop_active(channel, fade_out);
        if muted {
            return;
        }
        if let Some(slot) = self
            .start_stream("se", name, volume, pan, looping, fade_in, SlotKind::Other)
        {
            self.active.borrow_mut()[channel] = Some(slot);
        }
    }
    fn stop_se(&mut self, se2: bool, fade: FadeTicks) {
        self.stop_active(if se2 { ACTIVE_SE2 } else { ACTIVE_SE1 }, fade);
    }
    fn play_voice(&mut self, name: &str, volume: i64, pan: i32) {
        self.stop_active(ACTIVE_VOICE, None);
        if let Some(slot) = self
            .start_stream("voice", name, volume, pan, false, None, SlotKind::Voice)
        {
            self.active.borrow_mut()[ACTIVE_VOICE] = Some(slot);
        }
    }
    fn stop_voice(&mut self) {
        self.stop_active(ACTIVE_VOICE, None);
    }
    fn play_aux(&mut self, name: &str, volume: i64, channel: AuxChannel) {
        let slot = match channel {
            AuxChannel::Ui => AUX_UI,
            AuxChannel::TestBgm => AUX_TEST_BGM,
            AuxChannel::TestVoice => AUX_TEST_VOICE,
            AuxChannel::TestSe => AUX_TEST_SE,
        };
        let Some(bytes) = self.fetch("sys", name) else {
            log::warn!("系统 WAV 缺失: {name}");
            return;
        };
        self.shared.stop(&self.tx, slot);
        self.shared.start(&self.tx, slot, bytes, false, volume, 0, None, SlotKind::Aux);
        self.names.borrow_mut()[slot] = Some(name.to_string());
    }
    fn stop_all(&mut self) {
        self.shared.stop_all(&self.tx);
        *self.names.borrow_mut() = std::array::from_fn(|_| None);
        *self.active.borrow_mut() = [None; 4];
    }
    fn start_movie_audio(&mut self, stream: MovieAudioStream, volume: i64) -> bool {
        self.shared.stop(&self.tx, AUX_RESERVED);
        self.shared.attach_external(AUX_RESERVED, &stream, volume);
        true
    }
    fn stop_movie_audio(&mut self) {
        self.shared.stop(&self.tx, AUX_RESERVED);
    }
    fn bgm_elapsed_secs(&self) -> Option<f32> {
        let slot = self.active_slot(ACTIVE_BGM)?;
        self.shared.slot_alive(slot).then(|| self.shared.elapsed_secs(slot)).flatten()
    }
    fn bgm_playing(&self) -> bool {
        self.active_slot(ACTIVE_BGM).is_some_and(|slot| self.shared.slot_alive(slot))
    }
    fn voice_playing(&self) -> bool {
        self.active_slot(ACTIVE_VOICE).is_some_and(|slot| self.shared.slot_alive(slot))
    }
    fn voice_level(&self) -> f32 {
        self.shared.voice_level()
    }
    fn spectrum(&self, out: &mut [f32]) {
        self.shared.spectrum(out)
    }
}
const _: () = assert!(SLOTS == mixer::STREAM_SLOTS + mixer::AUX_SLOTS);
