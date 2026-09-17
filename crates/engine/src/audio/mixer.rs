use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
pub const VOLUME_CB: [i32; 101] = [
    -10000,
    -4000,
    -3397,
    -3045,
    -2795,
    -2602,
    -2443,
    -2309,
    -2193,
    -2091,
    -2000,
    -1917,
    -1841,
    -1772,
    -1707,
    -1647,
    -1591,
    -1539,
    -1489,
    -1442,
    -1397,
    -1355,
    -1315,
    -1276,
    -1239,
    -1204,
    -1170,
    -1137,
    -1105,
    -1075,
    -1045,
    -1017,
    -989,
    -962,
    -937,
    -911,
    -887,
    -863,
    -840,
    -817,
    -795,
    -774,
    -753,
    -733,
    -713,
    -693,
    -674,
    -655,
    -637,
    -619,
    -602,
    -584,
    -567,
    -551,
    -535,
    -519,
    -503,
    -488,
    -473,
    -458,
    -443,
    -429,
    -415,
    -401,
    -387,
    -374,
    -360,
    -347,
    -334,
    -322,
    -309,
    -297,
    -285,
    -273,
    -261,
    -249,
    -238,
    -227,
    -215,
    -204,
    -193,
    -183,
    -172,
    -161,
    -151,
    -141,
    -131,
    -120,
    -111,
    -101,
    -91,
    -81,
    -72,
    -63,
    -53,
    -44,
    -35,
    -26,
    -17,
    -8,
    0,
];
pub fn volume_gain(percent: i64) -> f32 {
    let p = percent.clamp(0, 100) as usize;
    10f32.powf(VOLUME_CB[p] as f32 / 2000.0)
}
fn pan_gains(p: i32) -> (f32, f32) {
    let p = p.clamp(-100, 100);
    if p == 0 {
        return (1.0, 1.0);
    }
    let mag = 10f32.powf(VOLUME_CB[(100 - p.unsigned_abs()) as usize] as f32 / 2000.0);
    if p > 0 { (mag, 1.0) } else { (1.0, mag) }
}
pub const STREAM_SLOTS: usize = 10;
pub const AUX_SLOTS: usize = 5;
pub const SLOTS: usize = STREAM_SLOTS + AUX_SLOTS;
pub const AUX_UI: usize = STREAM_SLOTS;
pub const AUX_TEST_BGM: usize = STREAM_SLOTS + 1;
pub const AUX_TEST_VOICE: usize = STREAM_SLOTS + 2;
pub const AUX_TEST_SE: usize = STREAM_SLOTS + 3;
pub const AUX_RESERVED: usize = STREAM_SLOTS + 4;
const MOVIE_MAX_SAMPLES: usize = 192_000 * 2 * 3;
static NEXT_MOVIE_GENERATION: AtomicU64 = AtomicU64::new(1 << 48);
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Other,
    Bgm,
    Voice,
    Aux,
}
pub struct Pipe {
    pub buf: VecDeque<f32>,
    pub eof: bool,
    pub loop_: bool,
    pub src_rate: u32,
    pub frames_read: u64,
    pub generation: u64,
}
#[derive(Clone)]
pub struct MovieAudioStream {
    pipe: Arc<Mutex<Pipe>>,
    discarded: Arc<AtomicBool>,
}
impl MovieAudioStream {
    pub(crate) fn new() -> Self {
        Self {
            pipe: Arc::new(
                Mutex::new(Pipe {
                    buf: VecDeque::new(),
                    eof: false,
                    loop_: false,
                    src_rate: 44_100,
                    frames_read: 0,
                    generation: NEXT_MOVIE_GENERATION.fetch_add(1, Ordering::Relaxed),
                }),
            ),
            discarded: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) fn configure(&self, sample_rate: u32) {
        self.pipe.lock().unwrap().src_rate = sample_rate.max(1);
    }
    pub(crate) fn push_stereo(&self, samples: &[f32], cancel: &AtomicBool) -> bool {
        debug_assert!(samples.len().is_multiple_of(2));
        let mut offset = 0;
        while offset < samples.len() {
            if cancel.load(Ordering::Acquire) {
                return false;
            }
            if self.discarded.load(Ordering::Acquire) {
                return true;
            }
            let written = {
                let mut pipe = self.pipe.lock().unwrap();
                let available = MOVIE_MAX_SAMPLES.saturating_sub(pipe.buf.len());
                let count = available.min(samples.len() - offset) & !1;
                pipe.buf.extend(samples[offset..offset + count].iter().copied());
                count
            };
            offset += written;
            if written == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        true
    }
    pub(crate) fn finish(&self) {
        self.pipe.lock().unwrap().eof = true;
    }
    pub(crate) fn discard(&self) {
        self.discarded.store(true, Ordering::Release);
        let mut pipe = self.pipe.lock().unwrap();
        pipe.buf.clear();
        pipe.eof = true;
    }
    pub(crate) fn available_frames(&self) -> usize {
        self.pipe.lock().unwrap().buf.len() / 2
    }
    pub(crate) fn position(&self) -> std::time::Duration {
        let pipe = self.pipe.lock().unwrap();
        std::time::Duration::from_secs_f64(
            pipe.frames_read as f64 / pipe.src_rate.max(1) as f64,
        )
    }
    pub(crate) fn is_drained(&self) -> bool {
        let pipe = self.pipe.lock().unwrap();
        pipe.eof && pipe.buf.is_empty()
    }
}
struct FadeState {
    target: i64,
    step: i64,
    frames_to_step: f64,
    interval_frames: f64,
}
struct SlotCtl {
    pipe: Option<Arc<Mutex<Pipe>>>,
    volume: i64,
    pan: i32,
    fade: Option<FadeState>,
    kind: SlotKind,
    stopping: bool,
}
pub enum LoaderCmd {
    Play { slot: usize, bytes: Vec<u8>, loop_: bool },
    Stop { slot: usize },
}
const TAP_LEN: usize = 2048;
pub struct MixerShared {
    slots: Mutex<[SlotCtl; SLOTS]>,
    tap: Mutex<VecDeque<f32>>,
    voice_peak: AtomicU32,
    alive: [AtomicBool; SLOTS],
    next_generation: AtomicU64,
    pub device_rate: u32,
}
impl MixerShared {
    pub fn new(device_rate: u32) -> Self {
        let mk = || SlotCtl {
            pipe: None,
            volume: 100,
            pan: 0,
            fade: None,
            kind: SlotKind::Other,
            stopping: false,
        };
        MixerShared {
            slots: Mutex::new(std::array::from_fn(|_| mk())),
            tap: Mutex::new(VecDeque::with_capacity(TAP_LEN)),
            voice_peak: AtomicU32::new(0),
            alive: std::array::from_fn(|_| AtomicBool::new(false)),
            next_generation: AtomicU64::new(1),
            device_rate,
        }
    }
    pub fn alloc_stream(&self, cursor: &mut usize) -> Option<usize> {
        for n in 0..STREAM_SLOTS {
            let slot = (*cursor + 1 + n) % STREAM_SLOTS;
            if !self.alive[slot].load(Ordering::Acquire) {
                *cursor = slot;
                return Some(slot);
            }
        }
        None
    }
    pub fn aux_slot(&self, slot: usize) -> bool {
        slot >= STREAM_SLOTS && slot < SLOTS
    }
    pub fn start(
        &self,
        tx: &Sender<LoaderCmd>,
        slot: usize,
        bytes: Vec<u8>,
        loop_: bool,
        volume_pct: i64,
        pan: i32,
        fade_in: Option<i64>,
        kind: SlotKind,
    ) {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let pipe = Arc::new(
            Mutex::new(Pipe {
                buf: VecDeque::new(),
                eof: false,
                loop_,
                src_rate: 44100,
                frames_read: 0,
                generation,
            }),
        );
        {
            let mut slots = self.slots.lock().unwrap();
            let s = &mut slots[slot];
            s.pipe = Some(pipe);
            let target = volume_pct.clamp(0, 100);
            let fade_in = fade_in.filter(|step| *step > 0);
            s.volume = if fade_in.is_some() { 0 } else { target };
            s.pan = pan;
            s.fade = fade_in
                .map(|step| FadeState {
                    target,
                    step,
                    frames_to_step: 2.0 * self.device_rate as f64 / 100.0,
                    interval_frames: 2.0 * self.device_rate as f64 / 100.0,
                });
            s.kind = kind;
            s.stopping = false;
        }
        self.alive[slot].store(true, Ordering::Release);
        let _ = tx
            .send(LoaderCmd::Play {
                slot,
                bytes,
                loop_,
            });
    }
    pub fn attach_external(
        &self,
        slot: usize,
        stream: &MovieAudioStream,
        volume_pct: i64,
    ) {
        let mut slots = self.slots.lock().unwrap();
        let control = &mut slots[slot];
        control.pipe = Some(stream.pipe.clone());
        control.volume = volume_pct.clamp(0, 100);
        control.pan = 0;
        control.fade = None;
        control.kind = SlotKind::Aux;
        control.stopping = false;
        self.alive[slot].store(true, Ordering::Release);
    }
    pub fn stop(&self, tx: &Sender<LoaderCmd>, slot: usize) {
        self.slots.lock().unwrap()[slot].pipe = None;
        self.alive[slot].store(false, Ordering::Release);
        let _ = tx.send(LoaderCmd::Stop { slot });
    }
    pub fn stop_fade(
        &self,
        tx: &Sender<LoaderCmd>,
        slot: usize,
        interval_ticks: Option<i64>,
    ) {
        let Some(interval_ticks) = interval_ticks.filter(|n| *n > 0) else {
            self.stop(tx, slot);
            return;
        };
        let mut slots = self.slots.lock().unwrap();
        let s = &mut slots[slot];
        if s.pipe.is_none() {
            return;
        }
        s.stopping = true;
        let interval_frames = interval_ticks as f64 * self.device_rate as f64 / 100.0;
        s.fade = Some(FadeState {
            target: 0,
            step: -1,
            frames_to_step: interval_frames,
            interval_frames,
        });
    }
    pub fn stop_all(&self, tx: &Sender<LoaderCmd>) {
        for slot in 0..SLOTS {
            self.stop(tx, slot);
        }
    }
    pub fn set_volume(&self, slot: usize, percent: i64) {
        let s = &mut self.slots.lock().unwrap()[slot];
        s.volume = percent.clamp(0, 100);
        s.fade = None;
    }
    pub fn fade_volume(&self, slot: usize, percent: i64, interval_ticks: Option<i64>) {
        let mut slots = self.slots.lock().unwrap();
        let s = &mut slots[slot];
        let target = percent.clamp(0, 100);
        if let Some(ticks) = interval_ticks.filter(|n| *n > 0) {
            if s.volume != target {
                let interval_frames = ticks as f64 * self.device_rate as f64 / 100.0;
                s.fade = Some(FadeState {
                    target,
                    step: if target > s.volume { 1 } else { -1 },
                    frames_to_step: interval_frames,
                    interval_frames,
                });
            }
        } else {
            s.volume = target;
            s.fade = None;
        }
    }
    pub fn set_pan(&self, slot: usize, pan: i32) {
        self.slots.lock().unwrap()[slot].pan = pan.clamp(-100, 100);
    }
    pub fn slot_alive(&self, slot: usize) -> bool {
        self.alive[slot].load(Ordering::Acquire)
    }
    pub fn elapsed_secs(&self, slot: usize) -> Option<f32> {
        let slots = self.slots.lock().unwrap();
        let pipe = slots[slot].pipe.as_ref()?.lock().unwrap();
        Some(pipe.frames_read as f32 / pipe.src_rate as f32)
    }
    pub fn voice_level(&self) -> f32 {
        f32::from_bits(self.voice_peak.load(Ordering::Relaxed))
    }
    pub fn spectrum(&self, out: &mut [f32]) {
        const N: usize = 512;
        out.fill(0.0);
        let tap = self.tap.lock().unwrap();
        if tap.len() < N {
            return;
        }
        let mut re = [0f32; N];
        let mut im = [0f32; N];
        let base = tap.len() - N;
        for i in 0..N {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos();
            re[i] = tap[base + i] * w;
        }
        drop(tap);
        fft(&mut re, &mut im);
        let bands = out.len();
        let nyq = self.device_rate as f32 / 2.0;
        for k in 0..bands {
            let f0 = 40.0 * (nyq / 40.0f32).powf(k as f32 / bands as f32);
            let f1 = 40.0 * (nyq / 40.0f32).powf((k + 1) as f32 / bands as f32);
            let b0 = ((f0 / nyq) * (N / 2) as f32) as usize;
            let b1 = ((((f1 / nyq) * (N / 2) as f32) as usize) + 1)
                .min(N / 2)
                .max(b0 + 1);
            let mut mag = 0.0f32;
            for b in b0..b1 {
                mag = mag.max((re[b] * re[b] + im[b] * im[b]).sqrt());
            }
            let db = 20.0 * (mag * 4.0 + 1e-9).log10();
            out[k] = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        }
    }
    pub fn mix(&self, resamp: &mut ResampBank, data: &mut [f32]) {
        let mut slots = self.slots.lock().unwrap();
        let pipe_arcs: [Option<Arc<Mutex<Pipe>>>; SLOTS] = {
            let mut arcs: [Option<Arc<Mutex<Pipe>>>; SLOTS] = std::array::from_fn(|_| {
                None
            });
            for s in 0..SLOTS {
                arcs[s] = slots[s].pipe.clone();
            }
            arcs
        };
        let mut pipes: [Option<std::sync::MutexGuard<'_, Pipe>>; SLOTS] = std::array::from_fn(|
            _|
        None);
        for s in 0..SLOTS {
            pipes[s] = pipe_arcs[s].as_ref().map(|p| p.lock().unwrap());
        }
        let mut tap = self.tap.lock().unwrap();
        let mut voice_peak_peak = 0.0f32;
        for frame in data.chunks_mut(2) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            let mut bgm_mono = 0.0f32;
            for s in 0..SLOTS {
                let (Some(pipe), Some(ctl)) = (pipes[s].as_mut(), slots.get_mut(s)) else {
                    continue;
                };
                let mut finished = false;
                if let Some(fade) = ctl.fade.as_mut() {
                    fade.frames_to_step -= 1.0;
                    while fade.frames_to_step <= 0.0 {
                        ctl.volume = (ctl.volume + fade.step).clamp(0, 100);
                        if (fade.step > 0 && ctl.volume >= fade.target)
                            || (fade.step < 0 && ctl.volume <= fade.target)
                        {
                            ctl.volume = fade.target;
                            finished = true;
                            break;
                        }
                        fade.frames_to_step += fade.interval_frames;
                    }
                }
                if finished {
                    ctl.fade = None;
                    if ctl.stopping {
                        ctl.pipe = None;
                        ctl.stopping = false;
                        self.alive[s].store(false, Ordering::Release);
                        continue;
                    }
                }
                let rs = &mut resamp.0[s];
                if rs.generation != pipe.generation {
                    rs.pos = 0.0;
                    rs.prev = [0.0; 2];
                    rs.cur = [0.0; 2];
                    rs.generation = pipe.generation;
                }
                let ratio = pipe.src_rate as f64 / self.device_rate as f64;
                rs.pos += ratio;
                while rs.pos >= 1.0 {
                    rs.pos -= 1.0;
                    rs.prev = rs.cur;
                    if pipe.buf.len() >= 2 {
                        rs.cur = [
                            pipe.buf.pop_front().unwrap(),
                            pipe.buf.pop_front().unwrap(),
                        ];
                        pipe.frames_read += 1;
                    } else if pipe.eof {
                        rs.cur = [0.0, 0.0];
                        self.alive[s].store(false, Ordering::Release);
                        break;
                    } else {
                        rs.pos = 0.0;
                        break;
                    }
                }
                let t = rs.pos as f32;
                let (gl, gr) = pan_gains(ctl.pan);
                let g = volume_gain(ctl.volume);
                let sl = (rs.prev[0] + (rs.cur[0] - rs.prev[0]) * t) * g * gl;
                let sr = (rs.prev[1] + (rs.cur[1] - rs.prev[1]) * t) * g * gr;
                l += sl;
                r += sr;
                match ctl.kind {
                    SlotKind::Bgm => bgm_mono += (sl + sr) * 0.5,
                    SlotKind::Voice => {
                        voice_peak_peak = voice_peak_peak.max(sl.abs()).max(sr.abs());
                    }
                    SlotKind::Other | SlotKind::Aux => {}
                }
            }
            frame[0] = l.clamp(-1.0, 1.0);
            frame[1] = r.clamp(-1.0, 1.0);
            tap.push_back(bgm_mono);
            while tap.len() > TAP_LEN {
                tap.pop_front();
            }
        }
        let prev = f32::from_bits(self.voice_peak.load(Ordering::Relaxed));
        let decay = 0.5f32
            .powf(data.len() as f32 / 2.0 / (0.3 * self.device_rate as f32));
        let next = voice_peak_peak.max(prev * decay).min(1.0);
        self.voice_peak.store(next.to_bits(), Ordering::Relaxed);
    }
}
pub struct Resamp {
    pos: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    generation: u64,
}
impl Resamp {
    fn new() -> Self {
        Resamp {
            pos: 0.0,
            prev: [0.0; 2],
            cur: [0.0; 2],
            generation: 0,
        }
    }
}
pub struct ResampBank(pub [Resamp; SLOTS]);
impl ResampBank {
    pub fn new() -> Self {
        ResampBank(std::array::from_fn(|_| Resamp::new()))
    }
}
pub fn loader_loop(rx: std::sync::mpsc::Receiver<LoaderCmd>, shared: Arc<MixerShared>) {
    let mut chains: [Option<DecodeChain>; SLOTS] = std::array::from_fn(|_| None);
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(5)) {
            Ok(LoaderCmd::Play { slot, bytes, loop_ }) => {
                match DecodeChain::new(&bytes, loop_) {
                    Ok(ch) => {
                        chains[slot] = Some(ch);
                    }
                    Err(e) => {
                        log::warn!("音频解码打开失败: {e}");
                        let slots = shared.slots.lock().unwrap();
                        if let Some(p) = &slots[slot].pipe {
                            let mut pipe = p.lock().unwrap();
                            pipe.eof = true;
                        }
                        chains[slot] = None;
                    }
                }
            }
            Ok(LoaderCmd::Stop { slot }) => chains[slot] = None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        for slot in 0..SLOTS {
            if chains[slot].is_none() {
                continue;
            }
            let mut made_progress = false;
            loop {
                enum FillStep {
                    Continue,
                    Full,
                    Stopped,
                    Eof { loop_: bool, buffered: bool },
                }
                let step = {
                    let slots = shared.slots.lock().unwrap();
                    match &slots[slot].pipe {
                        Some(p) => {
                            let mut pipe = p.lock().unwrap();
                            if pipe.buf.len() >= 2 * 16384 {
                                FillStep::Full
                            } else {
                                let ch = chains[slot]
                                    .as_mut()
                                    .expect("chain checked above");
                                match ch.next_block() {
                                    Some((frames, rate)) => {
                                        if pipe.src_rate != rate {
                                            pipe.src_rate = rate;
                                        }
                                        pipe.buf.extend(frames);
                                        made_progress = true;
                                        FillStep::Continue
                                    }
                                    None => {
                                        FillStep::Eof {
                                            loop_: pipe.loop_,
                                            buffered: !pipe.buf.is_empty(),
                                        }
                                    }
                                }
                            }
                        }
                        None => FillStep::Stopped,
                    }
                };
                match step {
                    FillStep::Continue => continue,
                    FillStep::Full => break,
                    FillStep::Stopped => {
                        chains[slot] = None;
                        break;
                    }
                    FillStep::Eof {
                        loop_: true,
                        buffered,
                    } if made_progress || buffered => {
                        let bytes = chains[slot]
                            .as_ref()
                            .expect("chain checked above")
                            .bytes
                            .clone();
                        match DecodeChain::new(&bytes, true) {
                            Ok(ch) => {
                                chains[slot] = Some(ch);
                                made_progress = false;
                            }
                            Err(e) => {
                                log::warn!("循环音频重建失败: {e}");
                                if let Some(p) = &shared.slots.lock().unwrap()[slot].pipe {
                                    p.lock().unwrap().eof = true;
                                }
                                chains[slot] = None;
                                break;
                            }
                        }
                    }
                    FillStep::Eof { .. } => {
                        if let Some(p) = &shared.slots.lock().unwrap()[slot].pipe {
                            p.lock().unwrap().eof = true;
                        }
                        chains[slot] = None;
                        break;
                    }
                }
            }
        }
    }
}
struct DecodeChain {
    bytes: Vec<u8>,
    #[allow(dead_code)]
    loop_: bool,
    reader: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    src_rate: u32,
    channels: usize,
    sample_buf: symphonia::core::audio::SampleBuffer<f32>,
}
impl DecodeChain {
    fn new(bytes: &[u8], loop_: bool) -> anyhow::Result<Self> {
        use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;
        let mss = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(bytes.to_vec())),
            Default::default(),
        );
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow::anyhow!("无音频轨"))?;
        let track_id = track.id;
        let params = track.codec_params.clone();
        let src_rate = params.sample_rate.unwrap_or(44100);
        let channels = params.channels.map(|c| c.count()).unwrap_or(2).max(1);
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())?;
        let stereo = symphonia::core::audio::SignalSpec::new(
            src_rate,
            symphonia::core::audio::Channels::FRONT_LEFT
                | symphonia::core::audio::Channels::FRONT_RIGHT,
        );
        Ok(DecodeChain {
            bytes: bytes.to_vec(),
            loop_,
            reader: format,
            decoder,
            track_id,
            src_rate,
            channels,
            sample_buf: symphonia::core::audio::SampleBuffer::new(5760, stereo),
        })
    }
    fn next_block(&mut self) -> Option<(Vec<f32>, u32)> {
        use symphonia::core::errors::Error as SymError;
        loop {
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                Err(_) => return None,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    if (spec.rate as u32) != self.src_rate
                        || spec.channels.count() != self.channels
                    {
                        self.src_rate = spec.rate;
                        self.channels = spec.channels.count();
                        self.sample_buf = symphonia::core::audio::SampleBuffer::new(
                            5760,
                            spec,
                        );
                    }
                    if decoded.frames() == 0 {
                        continue;
                    }
                    self.sample_buf.copy_interleaved_ref(decoded);
                    let src: &[f32] = self.sample_buf.samples();
                    let ch = self.channels;
                    let mut out = Vec::with_capacity(src.len() / ch * 2);
                    for fr in src.chunks_exact(ch) {
                        let l = fr[0];
                        let r = if ch >= 2 { fr[ch - 1] } else { l };
                        out.push(l);
                        out.push(r);
                    }
                    return Some((out, self.src_rate));
                }
                Err(SymError::DecodeError(_)) => continue,
                Err(_) => return None,
            }
        }
    }
}
fn fft(re: &mut [f32; 512], im: &mut [f32; 512]) {
    const N: usize = 512;
    let mut j = 0usize;
    for i in 0..N - 1 {
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
        let mut m = N >> 1;
        while m >= 1 && j & m != 0 {
            j &= !m;
            m >>= 1;
        }
        j |= m;
    }
    let mut len = 2;
    while len <= N {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < N {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (i + k, i + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

