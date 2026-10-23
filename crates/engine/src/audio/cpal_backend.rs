use super::mixer::{
    self, LoaderCmd, MixerShared, ResampBank, SlotKind, AUX_RESERVED, AUX_TEST_BGM,
    AUX_TEST_SE, AUX_TEST_VOICE, AUX_UI, SLOTS,
};
use super::{AudioBackend, AuxChannel, FadeTicks, MovieAudioStream};
use crate::vfs::Vfs;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use anyhow::Context;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;
const MIX_CHUNK_FRAMES: usize = 1024;
struct DeviceMixer {
    shared: Arc<MixerShared>,
    bank: ResampBank,
    channels: usize,
    stereo: [f32; MIX_CHUNK_FRAMES * 2],
}
impl DeviceMixer {
    fn new(shared: Arc<MixerShared>, channels: usize) -> Self {
        Self { shared, bank: ResampBank::new(), channels, stereo: [0.0; MIX_CHUNK_FRAMES * 2] }
    }
    fn render_f32(&mut self, output: &mut [f32]) {
        if self.channels == 2 {
            self.shared.mix(&mut self.bank, output);
        } else {
            self.render_converted(output, 0.0, |sample| sample);
        }
    }
    fn render_converted<T: Copy>(
        &mut self, output: &mut [T], silence: T, convert: impl Fn(f32) -> T + Copy,
    ) {
        for chunk in output.chunks_mut(self.channels * MIX_CHUNK_FRAMES) {
            let frames = chunk.len() / self.channels;
            let stereo = &mut self.stereo[..frames * 2];
            self.shared.mix(&mut self.bank, stereo);
            scatter_stereo(stereo, chunk, self.channels, silence, convert);
        }
    }
}
fn scatter_stereo<T: Copy>(
    stereo: &[f32], output: &mut [T], channels: usize, silence: T, convert: impl Fn(f32) -> T,
) {
    for (source, destination) in stereo.chunks_exact(2).zip(output.chunks_exact_mut(channels)) {
        destination[0] = convert(source[0]);
        destination[1] = convert(source[1]);
        destination[2..].fill(silence);
    }
    let remainder = output.len() / channels * channels;
    output[remainder..].fill(silence);
}
const ACTIVE_BGM: usize = 0;
const ACTIVE_VOICE: usize = 1;
const ACTIVE_SE1: usize = 2;
const ACTIVE_SE2: usize = 3;
pub struct CpalAudio {
    shared: Arc<MixerShared>,
    tx: Sender<LoaderCmd>,
    _stream: Option<cpal::Stream>,
    loader: Option<std::thread::JoinHandle<()>>,
    vfs: Rc<Vfs>,
    names: RefCell<[Option<String>; SLOTS]>,
    active: RefCell<[Option<usize>; 4]>,
    cursor: usize,
}
impl CpalAudio {
    pub fn new(vfs: Rc<Vfs>) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no default native audio output device")?;
        let mut stereo_f32 = None;
        let mut stereo_i16 = None;
        for range in device.supported_output_configs().context("enumerating native audio output configurations")? {
            if range.channels() == 2 {
                match range.sample_format() {
                    cpal::SampleFormat::F32 if stereo_f32.is_none() => stereo_f32 = Some(range),
                    cpal::SampleFormat::I16 if stereo_i16.is_none() => stereo_i16 = Some(range),
                    _ => {}
                }
            }
        }
        let supported = if let Some(range) = stereo_f32.or(stereo_i16) {
            let rate = 44100u32.clamp(range.min_sample_rate().0, range.max_sample_rate().0);
            range.with_sample_rate(cpal::SampleRate(rate))
        } else {
            // Keep the same real default device when it exposes only a
            // multichannel format. The native mixer remains stereo.
            device.default_output_config().context("reading native default audio output configuration")?
        };
        let channels = usize::from(supported.channels());
        anyhow::ensure!(channels >= 2, "native audio output has {channels} channels; stereo output requires at least two");
        let sample_format = supported.sample_format();
        let config = supported.config();
        let shared = Arc::new(MixerShared::new(config.sample_rate.0));
        let (tx, rx) = std::sync::mpsc::channel::<LoaderCmd>();
        let err = |e| log::warn!("音频设备错误: {e}");
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut mixer = DeviceMixer::new(Arc::clone(&shared), channels);
                device.build_output_stream(
                    &config, move |data: &mut [f32], _| mixer.render_f32(data), err, None,
                ).with_context(|| format!("creating native F32 output stream ({channels} channels at {} Hz)", config.sample_rate.0))?
            }
            cpal::SampleFormat::I16 => {
                let mut mixer = DeviceMixer::new(Arc::clone(&shared), channels);
                device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| mixer.render_converted(data, 0, |sample| {
                        (sample * 32767.0).clamp(-32768.0, 32767.0) as i16
                    }),
                    err, None,
                ).with_context(|| format!("creating native I16 output stream ({channels} channels at {} Hz)", config.sample_rate.0))?
            }
            cpal::SampleFormat::I32 => {
                let mut mixer = DeviceMixer::new(Arc::clone(&shared), channels);
                device.build_output_stream(
                    &config,
                    move |data: &mut [i32], _| mixer.render_converted(data, 0, |sample| {
                        (f64::from(sample) * f64::from(i32::MAX)).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
                    }),
                    err, None,
                ).with_context(|| format!("creating native I32 output stream ({channels} channels at {} Hz)", config.sample_rate.0))?
            }
            format => anyhow::bail!("unsupported native audio output format {format:?} for {channels} channels"),
        };
        stream.play().context("starting native audio output stream")?;
        let loader = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("eden-audio-loader".into())
                .spawn(move || mixer::loader_loop(rx, shared))
                .context("starting native audio loader thread")?
        };
        log::info!("音频输出: default @ {}Hz {sample_format:?} channels={channels}", config.sample_rate.0);
        Ok(CpalAudio {
            shared, tx, _stream: Some(stream), loader: Some(loader), vfs,
            names: RefCell::new(std::array::from_fn(|_| None)),
            active: RefCell::new([None; 4]), cursor: mixer::STREAM_SLOTS - 1,
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
impl Drop for CpalAudio {
    fn drop(&mut self) {
        self._stream.take();
        let (replacement, _) = std::sync::mpsc::channel();
        drop(std::mem::replace(&mut self.tx, replacement));
        if let Some(loader) = self.loader.take() {
            let _ = loader.join();
        }
    }
}
impl AudioBackend for CpalAudio {
    fn set_paused(&mut self, paused: bool) -> anyhow::Result<()> {
        if let Some(stream) = &self._stream {
            if paused { stream.pause()?; } else { stream.play()?; }
        }
        Ok(())
    }
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
#[cfg(test)]
mod device_layout_tests {
    use super::*;
    #[test]
    fn stereo_maps_to_front_channels_and_clears_surround_channels() {
        let mut output = [9.0; 16];
        scatter_stereo(&[0.25, -0.5, 0.75, -1.0], &mut output, 8, 0.0, |value| value);
        assert_eq!(&output[..8], &[0.25, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(&output[8..], &[0.75, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
}
