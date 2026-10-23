use crate::audio::{AudioBackend, MovieAudioStream};
use crate::vfs::MovieSource;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use web_time::Instant;
const VIDEO_CHANNEL_CAPACITY: usize = 4;
const VIDEO_REORDER_DEPTH: usize = 4;
const HOST_FRAME_CAPACITY: usize = 4;
const AUDIO_PREBUFFER: Duration = Duration::from_millis(120);
const VIDEO_DUE_TOLERANCE: Duration = Duration::from_millis(5);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioFormat {
    channels: u16,
    sample_rate: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieColorMatrix {
    Bt601Limited,
    Bt709Limited,
}
#[derive(Clone, Debug)]
pub enum MoviePixels {
    Nv12 { luma: Arc<Vec<u8>>, chroma: Arc<Vec<u8>> },
    Bgra8(Arc<Vec<u8>>),
    Rgba8Texture(Arc<wgpu::Texture>),
}
#[derive(Clone, Debug)]
pub struct MovieFrame {
    pub revision: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: MoviePixels,
    pub matrix: MovieColorMatrix,
    pub rect: [f32; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct MovieTarget {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieStatus {
    Idle,
    Playing,
    Finished,
    Failed,
}
pub trait VideoBackend {
    fn play(
        &mut self,
        source: MovieSource,
        target: MovieTarget,
        audio: &mut dyn AudioBackend,
    );
    fn poll(&mut self, audio: &mut dyn AudioBackend, volume: i64) -> MovieStatus;
    fn current_frame(&self) -> Option<MovieFrame>;
    fn cleanup(&mut self, audio: &mut dyn AudioBackend);
}
#[derive(Debug)]
struct DecodedFrame {
    pts: Duration,
    end_pts: Duration,
    width: u32,
    height: u32,
    pixels: MoviePixels,
}
#[derive(Debug)]
enum DecoderEvent {
    Ready { width: u32, height: u32, audio: Option<AudioFormat> },
    Finished { end_pts: Duration },
    Failed(String),
}
struct MovieSession {
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    event_rx: mpsc::Receiver<DecoderEvent>,
    frame_rx: mpsc::Receiver<DecodedFrame>,
    frame_channel_closed: bool,
    frames: VecDeque<DecodedFrame>,
    audio_stream: MovieAudioStream,
    audio_format: Option<AudioFormat>,
    audio_started: bool,
    audio_output: bool,
    fallback_clock: Option<Instant>,
    current_frame: Option<MovieFrame>,
    target: MovieTarget,
    decoder_ready: bool,
    decoder_finished: bool,
    failed: bool,
    latest_end_pts: Duration,
}
impl MovieSession {
    fn playback_position(&self) -> Duration {
        if self.audio_output {
            self.audio_stream.position()
        } else {
            self.fallback_clock.map(|started| started.elapsed()).unwrap_or_default()
        }
    }
    fn stop(self, audio: &mut dyn AudioBackend) {
        self.cancel.store(true, Ordering::Release);
        self.audio_stream.discard();
        audio.stop_movie_audio();
    }
}
impl Drop for MovieSession {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.audio_stream.discard();
        // Disconnect before joining: a full frame queue may block the decoder.
        let (_, empty) = mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.frame_rx, empty));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub struct NativeVideoBackend {
    next_revision: u64,
    session: Option<MovieSession>,
}
impl NativeVideoBackend {
    pub fn new() -> Self {
        Self {
            next_revision: 1,
            session: None,
        }
    }
    pub(crate) fn play(
        &mut self,
        source: MovieSource,
        target: MovieTarget,
        audio: &mut dyn AudioBackend,
    ) {
        self.cleanup(audio);
        let cancel = Arc::new(AtomicBool::new(false));
        let audio_stream = MovieAudioStream::new();
        let (event_tx, event_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::sync_channel(VIDEO_CHANNEL_CAPACITY);
        let worker = spawn_decoder(source, cancel.clone(), audio_stream.clone(), event_tx, frame_tx);
        self.session = Some(MovieSession {
            cancel,
            worker,
            event_rx,
            frame_rx,
            frame_channel_closed: false,
            frames: VecDeque::new(),
            audio_stream,
            audio_format: None,
            audio_started: false,
            audio_output: false,
            fallback_clock: None,
            current_frame: None,
            target,
            decoder_ready: false,
            decoder_finished: false,
            failed: false,
            latest_end_pts: Duration::ZERO,
        });
    }
    pub(crate) fn poll(
        &mut self,
        audio: &mut dyn AudioBackend,
        volume: i64,
    ) -> MovieStatus {
        let Some(session) = self.session.as_mut() else {
            return MovieStatus::Idle;
        };
        while let Ok(event) = session.event_rx.try_recv() {
            match event {
                DecoderEvent::Ready { width, height, audio } => {
                    session.decoder_ready = true;
                    session.audio_format = audio;
                    log::info!("电影解码就绪: {width}x{height}, audio={audio:?}");
                    if audio.is_none() {
                        session.fallback_clock = Some(Instant::now());
                    }
                }
                DecoderEvent::Finished { end_pts } => {
                    session.decoder_finished = true;
                    session.latest_end_pts = session.latest_end_pts.max(end_pts);
                    session.audio_stream.finish();
                }
                DecoderEvent::Failed(error) => {
                    log::error!("电影解码失败: {error}");
                    session.failed = true;
                    session.decoder_finished = true;
                    session.audio_stream.finish();
                }
            }
        }
        while session.frames.len() < HOST_FRAME_CAPACITY {
            match session.frame_rx.try_recv() {
                Ok(frame) => {
                    session.latest_end_pts = session.latest_end_pts.max(frame.end_pts);
                    session.frames.push_back(frame);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    session.frame_channel_closed = true;
                    session.decoder_finished = true;
                    break;
                }
            }
        }
        if !session.audio_started {
            if let Some(format) = session.audio_format {
                let prebuffer_frames = (format.sample_rate as f64
                    * AUDIO_PREBUFFER.as_secs_f64()) as usize;
                if session.audio_stream.available_frames() >= prebuffer_frames
                    || session.decoder_finished
                {
                    session.audio_started = true;
                    session.audio_output = audio
                        .start_movie_audio(
                            session.audio_stream.clone(),
                            volume.clamp(0, 100),
                        );
                    if !session.audio_output {
                        session.audio_stream.discard();
                        session.fallback_clock = Some(Instant::now());
                    }
                }
            } else if session.decoder_ready {
                session.fallback_clock.get_or_insert_with(Instant::now);
            }
        }
        let mut clock = session.playback_position();
        if session.decoder_finished && session.audio_output
            && session.audio_stream.is_drained()
        {
            clock = clock.max(session.latest_end_pts);
        }
        while session
            .frames
            .front()
            .is_some_and(|frame| frame.pts <= clock + VIDEO_DUE_TOLERANCE)
        {
            let frame = session.frames.pop_front().unwrap();
            let output_width = if session.target.width > 0 {
                session.target.width as f32
            } else {
                frame.width as f32
            };
            let output_height = if session.target.height > 0 {
                session.target.height as f32
            } else {
                frame.height as f32
            };
            session.current_frame = Some(MovieFrame {
                revision: self.next_revision,
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels,
                matrix: if frame.height >= 720 {
                    MovieColorMatrix::Bt709Limited
                } else {
                    MovieColorMatrix::Bt601Limited
                },
                rect: [
                    session.target.x as f32,
                    session.target.y as f32,
                    output_width,
                    output_height,
                ],
            });
            self.next_revision = self.next_revision.wrapping_add(1).max(1);
        }
        if session.failed {
            return MovieStatus::Failed;
        }
        let frames_done = session.frames.is_empty() && session.frame_channel_closed;
        let timeline_done = if session.audio_output {
            session.audio_stream.is_drained()
        } else {
            clock >= session.latest_end_pts
        };
        if session.decoder_finished && frames_done && timeline_done {
            MovieStatus::Finished
        } else {
            MovieStatus::Playing
        }
    }
    pub(crate) fn current_frame(&self) -> Option<MovieFrame> {
        self.session.as_ref().and_then(|session| session.current_frame.clone())
    }
    pub(crate) fn cleanup(&mut self, audio: &mut dyn AudioBackend) {
        if let Some(session) = self.session.take() {
            session.stop(audio);
        }
    }
}
impl Default for NativeVideoBackend {
    fn default() -> Self {
        Self::new()
    }
}
impl VideoBackend for NativeVideoBackend {
    fn play(
        &mut self,
        source: MovieSource,
        target: MovieTarget,
        audio: &mut dyn AudioBackend,
    ) {
        NativeVideoBackend::play(self, source, target, audio);
    }
    fn poll(&mut self, audio: &mut dyn AudioBackend, volume: i64) -> MovieStatus {
        NativeVideoBackend::poll(self, audio, volume)
    }
    fn current_frame(&self) -> Option<MovieFrame> {
        NativeVideoBackend::current_frame(self)
    }
    fn cleanup(&mut self, audio: &mut dyn AudioBackend) {
        NativeVideoBackend::cleanup(self, audio);
    }
}
#[derive(Default)]
pub struct NullVideoBackend {
    skipped: Option<String>,
    pending_finish: bool,
}
impl NullVideoBackend {
    pub fn skipped(&self) -> Option<&str> {
        self.skipped.as_deref()
    }
}
impl VideoBackend for NullVideoBackend {
    fn play(
        &mut self,
        source: MovieSource,
        _target: MovieTarget,
        _audio: &mut dyn AudioBackend,
    ) {
        let name = match source {
            MovieSource::File(path) => {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<invalid movie name>")
                    .to_owned()
            }
            MovieSource::Archive { name, .. } => name,
            MovieSource::Unavailable(name) => name,
        };
        log::warn!("Web 视频暂不支持，已跳过: {name}");
        self.skipped = Some(name);
        self.pending_finish = true;
    }
    fn poll(&mut self, _audio: &mut dyn AudioBackend, _volume: i64) -> MovieStatus {
        if std::mem::take(&mut self.pending_finish) {
            MovieStatus::Finished
        } else {
            MovieStatus::Idle
        }
    }
    fn current_frame(&self) -> Option<MovieFrame> {
        None
    }
    fn cleanup(&mut self, _audio: &mut dyn AudioBackend) {
        self.pending_finish = false;
    }
}
fn spawn_decoder(
    source: MovieSource,
    cancel: Arc<AtomicBool>,
    audio_stream: MovieAudioStream,
    event_tx: mpsc::Sender<DecoderEvent>,
    frame_tx: mpsc::SyncSender<DecodedFrame>,
) -> Option<thread::JoinHandle<()>> {
    let spawn_error_tx = event_tx.clone();
    let result = thread::Builder::new()
        .name("eden-movie-decoder".into())
        .spawn(move || {
            let result = source
                .materialize()
                .and_then(|path| {
                    platform::decode(&path, &cancel, &audio_stream, &event_tx, &frame_tx)
                });
            if let Err(error) = result {
                audio_stream.finish();
                let _ = event_tx.send(DecoderEvent::Failed(error));
            }
        });
    match result {
        Ok(worker) => Some(worker),
        Err(error) => {
            let _ = spawn_error_tx.send(DecoderEvent::Failed(
                format!("cannot spawn movie decoder thread: {error}"),
            ));
            None
        }
    }
}
#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED,
    };
    struct MediaFoundationGuard {
        com_initialized: bool,
        mf_started: bool,
    }
    impl MediaFoundationGuard {
        unsafe fn start() -> Result<Self, String> {
            let mut guard = Self {
                com_initialized: false,
                mf_started: false,
            };
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|error| format!("CoInitializeEx: {error}"))?;
            guard.com_initialized = true;
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|error| format!("MFStartup: {error}"))?;
            guard.mf_started = true;
            Ok(guard)
        }
    }
    impl Drop for MediaFoundationGuard {
        fn drop(&mut self) {
            unsafe {
                if self.mf_started {
                    let _ = MFShutdown();
                }
                if self.com_initialized {
                    CoUninitialize();
                }
            }
        }
    }
    struct VideoInfo {
        stream: u32,
        width: u32,
        height: u32,
        stride: i32,
        format: VideoPixelFormat,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum VideoPixelFormat {
        Nv12,
        Bgra8,
    }
    struct AudioInfo {
        stream: u32,
        format: AudioFormat,
        bits_per_sample: u32,
    }
    pub(super) fn decode(
        path: &Path,
        cancel: &AtomicBool,
        audio_stream: &MovieAudioStream,
        event_tx: &mpsc::Sender<DecoderEvent>,
        frame_tx: &mpsc::SyncSender<DecodedFrame>,
    ) -> Result<(), String> {
        unsafe {
            let _guard = MediaFoundationGuard::start()?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 3)
                .map_err(|error| format!("MFCreateAttributes: {error}"))?;
            let attributes = attributes.ok_or("MFCreateAttributes returned null")?;
            attributes
                .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
                .map_err(|error| format!("enable video processing: {error}"))?;
            attributes
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(|error| format!("enable hardware transforms: {error}"))?;
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let reader = MFCreateSourceReaderFromURL(
                    PCWSTR(wide_path.as_ptr()),
                    Some(&attributes),
                )
                .map_err(|error| format!("open {}: {error}", path.display()))?;
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(|error| format!("deselect streams: {error}"))?;
            let (video_stream, audio_stream_index) = discover_streams(&reader)?;
            let mut video = configure_video(&reader, video_stream)?;
            let mut audio = audio_stream_index
                .map(|stream| configure_audio(&reader, stream))
                .transpose()?;
            if let Some(info) = &audio {
                audio_stream.configure(info.format.sample_rate);
            }
            event_tx
                .send(DecoderEvent::Ready {
                    width: video.width,
                    height: video.height,
                    audio: audio.as_ref().map(|info| info.format),
                })
                .map_err(|_| {
                    "movie session closed before decoder was ready".to_owned()
                })?;
            let mut end_pts = Duration::ZERO;
            let mut reordered_frames = Vec::with_capacity(VIDEO_REORDER_DEPTH + 1);
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(());
                }
                let mut actual_stream = 0;
                let mut flags = 0;
                let mut timestamp_hns = 0i64;
                let mut sample = None;
                reader
                    .ReadSample(
                        MF_SOURCE_READER_ANY_STREAM.0 as u32,
                        0,
                        Some(&mut actual_stream),
                        Some(&mut flags),
                        Some(&mut timestamp_hns),
                        Some(&mut sample),
                    )
                    .map_err(|error| format!("ReadSample: {error}"))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    return Err(
                        format!(
                            "source reader reported an error on stream {actual_stream}"
                        ),
                    );
                }
                if flags & MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32 != 0 {
                    if actual_stream == video.stream {
                        video = configure_video(&reader, video.stream)?;
                    } else if audio
                        .as_ref()
                        .is_some_and(|info| actual_stream == info.stream)
                    {
                        let current = configure_audio(&reader, actual_stream)?;
                        ensure_stable_audio_format(audio.as_ref(), &current)?;
                        audio = Some(current);
                    }
                } else if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0
                {
                    if actual_stream == video.stream {
                        video = query_video_info(&reader, video.stream)?;
                    } else if audio
                        .as_ref()
                        .is_some_and(|info| actual_stream == info.stream)
                    {
                        let current = query_audio_info(&reader, actual_stream)?;
                        ensure_stable_audio_format(audio.as_ref(), &current)?;
                        audio = Some(current);
                    }
                }
                let Some(sample) = sample else {
                    continue;
                };
                let pts = hns_to_duration(timestamp_hns);
                let duration = sample
                    .GetSampleDuration()
                    .ok()
                    .map(hns_to_duration)
                    .unwrap_or_default();
                end_pts = end_pts.max(pts + duration);
                if actual_stream == video.stream {
                    let frame = DecodedFrame {
                        pts,
                        end_pts: pts + duration,
                        width: video.width,
                        height: video.height,
                        pixels: copy_video_sample(&sample, &video)?,
                    };
                    let insertion = reordered_frames
                        .partition_point(|queued: &DecodedFrame| {
                            queued.pts <= frame.pts
                        });
                    reordered_frames.insert(insertion, frame);
                    if reordered_frames.len() > VIDEO_REORDER_DEPTH {
                        let frame = reordered_frames.remove(0);
                        if frame_tx.send(frame).is_err() {
                            return Ok(());
                        }
                    }
                } else if let Some(info) = &audio {
                    if actual_stream == info.stream {
                        let samples = copy_audio_sample_stereo(&sample, info)?;
                        if !audio_stream.push_stereo(&samples, cancel) {
                            return Ok(());
                        }
                    }
                }
            }
            for frame in reordered_frames {
                if cancel.load(Ordering::Acquire) || frame_tx.send(frame).is_err() {
                    return Ok(());
                }
            }
            audio_stream.finish();
            let _ = event_tx.send(DecoderEvent::Finished { end_pts });
            Ok(())
        }
    }
    unsafe fn discover_streams(
        reader: &IMFSourceReader,
    ) -> Result<(u32, Option<u32>), String> {
        let mut video = None;
        let mut audio = None;
        for stream in 0..32u32 {
            let Ok(native) = reader.GetNativeMediaType(stream, 0) else {
                if stream > 1 && (video.is_some() || audio.is_some()) {
                    break;
                }
                continue;
            };
            let Ok(major) = native.GetGUID(&MF_MT_MAJOR_TYPE) else {
                continue;
            };
            if major == MFMediaType_Video && video.is_none() {
                video = Some(stream);
            } else if major == MFMediaType_Audio && audio.is_none() {
                audio = Some(stream);
            }
        }
        video
            .map(|stream| (stream, audio))
            .ok_or_else(|| "movie has no decodable video stream".into())
    }
    unsafe fn configure_video(
        reader: &IMFSourceReader,
        stream: u32,
    ) -> Result<VideoInfo, String> {
        let format = if set_video_output_type(reader, stream, &MFVideoFormat_NV12)
            .is_ok()
        {
            VideoPixelFormat::Nv12
        } else {
            set_video_output_type(reader, stream, &MFVideoFormat_RGB32)
                .map_err(|error| format!("configure NV12/RGB32 decoder: {error}"))?;
            VideoPixelFormat::Bgra8
        };
        reader
            .SetStreamSelection(stream, true)
            .map_err(|error| format!("select video stream: {error}"))?;
        let info = query_video_info(reader, stream)?;
        log::info!(
            "电影解码输出: {:?} {}x{} stride={}", format, info.width, info.height,
            info.stride
        );
        Ok(info)
    }
    unsafe fn set_video_output_type(
        reader: &IMFSourceReader,
        stream: u32,
        subtype: &windows::core::GUID,
    ) -> windows::core::Result<()> {
        let media_type = MFCreateMediaType()?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        reader.SetCurrentMediaType(stream, None, &media_type)
    }
    unsafe fn query_video_info(
        reader: &IMFSourceReader,
        stream: u32,
    ) -> Result<VideoInfo, String> {
        let current = reader
            .GetCurrentMediaType(stream)
            .map_err(|error| format!("query video type: {error}"))?;
        let major = current
            .GetGUID(&MF_MT_MAJOR_TYPE)
            .map_err(|error| format!("query video major type: {error}"))?;
        let subtype = current
            .GetGUID(&MF_MT_SUBTYPE)
            .map_err(|error| format!("query video subtype: {error}"))?;
        if major != MFMediaType_Video {
            return Err(format!("unexpected decoded video major type: {major:?}"));
        }
        let format = if subtype == MFVideoFormat_NV12 {
            VideoPixelFormat::Nv12
        } else if subtype == MFVideoFormat_RGB32 {
            VideoPixelFormat::Bgra8
        } else {
            return Err(format!("unexpected decoded video subtype: {subtype:?}"));
        };
        let frame_size = current
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|error| format!("query video dimensions: {error}"))?;
        let width = (frame_size >> 32) as u32;
        let height = frame_size as u32;
        if width == 0 || height == 0 {
            return Err(format!("invalid movie dimensions {width}x{height}"));
        }
        let stride = current
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map(|value| value as i32)
            .or_else(|_| MFGetStrideForBitmapInfoHeader(subtype.data1, width))
            .unwrap_or(
                match format {
                    VideoPixelFormat::Nv12 => width as i32,
                    VideoPixelFormat::Bgra8 => (width * 4) as i32,
                },
            );
        Ok(VideoInfo {
            stream,
            width,
            height,
            stride,
            format,
        })
    }
    unsafe fn configure_audio(
        reader: &IMFSourceReader,
        stream: u32,
    ) -> Result<AudioInfo, String> {
        let media_type = MFCreateMediaType()
            .map_err(|error| format!("create audio type: {error}"))?;
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|error| format!("set audio major type: {error}"))?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
            .map_err(|error| format!("set PCM subtype: {error}"))?;
        reader
            .SetCurrentMediaType(stream, None, &media_type)
            .map_err(|error| format!("configure PCM decoder: {error}"))?;
        reader
            .SetStreamSelection(stream, true)
            .map_err(|error| format!("select audio stream: {error}"))?;
        query_audio_info(reader, stream)
    }
    unsafe fn query_audio_info(
        reader: &IMFSourceReader,
        stream: u32,
    ) -> Result<AudioInfo, String> {
        let current = reader
            .GetCurrentMediaType(stream)
            .map_err(|error| format!("query audio type: {error}"))?;
        let major = current
            .GetGUID(&MF_MT_MAJOR_TYPE)
            .map_err(|error| format!("query audio major type: {error}"))?;
        let subtype = current
            .GetGUID(&MF_MT_SUBTYPE)
            .map_err(|error| format!("query audio subtype: {error}"))?;
        if major != MFMediaType_Audio || subtype != MFAudioFormat_PCM {
            return Err(
                format!(
                    "unexpected decoded audio format: major={major:?} subtype={subtype:?}"
                ),
            );
        }
        let channels = current
            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
            .map_err(|error| format!("query audio channels: {error}"))?;
        let sample_rate = current
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .map_err(|error| format!("query audio sample rate: {error}"))?;
        let bits_per_sample = current
            .GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)
            .map_err(|error| format!("query audio bit depth: {error}"))?;
        let channels = u16::try_from(channels)
            .map_err(|_| "audio channel count is too large")?;
        if channels == 0 || sample_rate == 0 {
            return Err("invalid decoded audio format".into());
        }
        if !matches!(bits_per_sample, 8 | 16 | 24 | 32) {
            return Err(format!("unsupported PCM bit depth {bits_per_sample}"));
        }
        Ok(AudioInfo {
            stream,
            format: AudioFormat {
                channels,
                sample_rate,
            },
            bits_per_sample,
        })
    }
    fn ensure_stable_audio_format(
        previous: Option<&AudioInfo>,
        current: &AudioInfo,
    ) -> Result<(), String> {
        if previous
            .is_some_and(|previous| {
                previous.format != current.format
                    || previous.bits_per_sample != current.bits_per_sample
            })
        {
            let previous = previous.unwrap();
            return Err(
                format!(
                    "decoded audio format changed: {:?}/{}-bit -> {:?}/{}-bit", previous
                    .format, previous.bits_per_sample, current.format, current
                    .bits_per_sample
                ),
            );
        }
        Ok(())
    }
    unsafe fn copy_video_sample(
        sample: &IMFSample,
        info: &VideoInfo,
    ) -> Result<MoviePixels, String> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|error| format!("make video buffer contiguous: {error}"))?;
        if info.format == VideoPixelFormat::Nv12 {
            let mut data = std::ptr::null_mut();
            let mut length = 0u32;
            buffer
                .Lock(&mut data, None, Some(&mut length))
                .map_err(|error| format!("lock NV12 buffer: {error}"))?;
            let result = copy_nv12_rows(
                data,
                length as usize,
                info.stride,
                info.width,
                info.height,
            );
            let _ = buffer.Unlock();
            return result
                .map(|(luma, chroma)| MoviePixels::Nv12 {
                    luma: Arc::new(luma),
                    chroma: Arc::new(chroma),
                });
        }
        if let Ok(buffer_2d) = buffer.cast::<IMF2DBuffer>() {
            let mut scanline = std::ptr::null_mut();
            let mut stride = 0i32;
            buffer_2d
                .Lock2D(&mut scanline, &mut stride)
                .map_err(|error| format!("lock RGB32 buffer: {error}"))?;
            let result = copy_bgra_rows(scanline, stride, info.width, info.height);
            let _ = buffer_2d.Unlock2D();
            return result.map(|pixels| MoviePixels::Bgra8(Arc::new(pixels)));
        }
        let mut data = std::ptr::null_mut();
        let mut length = 0u32;
        buffer
            .Lock(&mut data, None, Some(&mut length))
            .map_err(|error| format!("lock contiguous RGB32 buffer: {error}"))?;
        let stride = if info.stride == 0 {
            (info.width * 4) as i32
        } else {
            info.stride
        };
        let required = stride.unsigned_abs() as usize * info.height as usize;
        let result = if required > length as usize {
            Err(format!("short RGB32 sample: {length} bytes, need {required}"))
        } else {
            let scanline = if stride < 0 {
                data.add((info.height as usize - 1) * stride.unsigned_abs() as usize)
            } else {
                data
            };
            copy_bgra_rows(scanline, stride, info.width, info.height)
        };
        let _ = buffer.Unlock();
        result.map(|pixels| MoviePixels::Bgra8(Arc::new(pixels)))
    }
    pub(super) unsafe fn copy_nv12_rows(
        data: *const u8,
        length: usize,
        stride: i32,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Vec<u8>), String> {
        if data.is_null() {
            return Err("NV12 buffer returned a null pointer".into());
        }
        if width == 0 || height == 0 || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
        {
            return Err(format!("invalid NV12 dimensions {width}x{height}"));
        }
        if stride <= 0 || (stride as u32) < width {
            return Err(format!("invalid NV12 stride {stride} for width {width}"));
        }
        let stride = stride as usize;
        let width = width as usize;
        let height = height as usize;
        let chroma_rows = height / 2;
        let required = stride
            .checked_mul(height + chroma_rows)
            .ok_or("NV12 buffer size overflow")?;
        if length < required {
            return Err(format!("short NV12 sample: {length} bytes, need {required}"));
        }
        let bytes = std::slice::from_raw_parts(data, required);
        let mut luma = vec![0u8; width * height];
        let mut chroma = vec![0u8; width * chroma_rows];
        for row in 0..height {
            luma[row * width..(row + 1) * width]
                .copy_from_slice(&bytes[row * stride..row * stride + width]);
        }
        let chroma_base = stride * height;
        for row in 0..chroma_rows {
            chroma[row * width..(row + 1) * width]
                .copy_from_slice(
                    &bytes[chroma_base
                        + row * stride..chroma_base + row * stride + width],
                );
        }
        Ok((luma, chroma))
    }
    unsafe fn copy_bgra_rows(
        scanline0: *const u8,
        stride: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        if scanline0.is_null() {
            return Err("RGB32 buffer returned a null scanline".into());
        }
        let row_bytes = width as usize * 4;
        if (stride.unsigned_abs() as usize) < row_bytes {
            return Err(format!("RGB32 stride {stride} is smaller than {row_bytes}"));
        }
        let mut bgra = vec![0u8; row_bytes * height as usize];
        for y in 0..height as usize {
            let source = scanline0.offset(y as isize * stride as isize);
            let source = std::slice::from_raw_parts(source, row_bytes);
            bgra[y * row_bytes..(y + 1) * row_bytes].copy_from_slice(source);
        }
        Ok(bgra)
    }
    unsafe fn copy_audio_sample_stereo(
        sample: &IMFSample,
        info: &AudioInfo,
    ) -> Result<Vec<f32>, String> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|error| format!("make audio buffer contiguous: {error}"))?;
        let mut data = std::ptr::null_mut();
        let mut length = 0u32;
        buffer
            .Lock(&mut data, None, Some(&mut length))
            .map_err(|error| format!("lock audio buffer: {error}"))?;
        let bytes = std::slice::from_raw_parts(data, length as usize);
        let mut decoded: Vec<f32> = match info.bits_per_sample {
            8 => bytes.iter().map(|sample| (*sample as f32 - 128.0) / 128.0).collect(),
            16 => {
                bytes
                    .chunks_exact(2)
                    .map(|sample| {
                        i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0
                    })
                    .collect()
            }
            24 => {
                bytes
                    .chunks_exact(3)
                    .map(|sample| {
                        let value = ((sample[0] as i32) | ((sample[1] as i32) << 8)
                            | ((sample[2] as i32) << 16)) << 8 >> 8;
                        value as f32 / 8_388_608.0
                    })
                    .collect()
            }
            32 => {
                bytes
                    .chunks_exact(4)
                    .map(|sample| {
                        i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
                            as f32 / 2_147_483_648.0
                    })
                    .collect()
            }
            _ => unreachable!(),
        };
        let _ = buffer.Unlock();
        let channels = info.format.channels as usize;
        decoded.truncate(decoded.len() / channels * channels);
        if channels == 2 {
            return Ok(decoded);
        }
        let mut stereo = Vec::with_capacity(decoded.len() / channels * 2);
        for frame in decoded.chunks_exact(channels) {
            if channels == 1 {
                stereo.extend_from_slice(&[frame[0], frame[0]]);
            } else {
                stereo.extend_from_slice(&[frame[0], frame[1]]);
            }
        }
        Ok(stereo)
    }
    fn hns_to_duration(value: i64) -> Duration {
        Duration::from_nanos(value.max(0) as u64 * 100)
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn decode(
        _path: &Path,
        _cancel: &AtomicBool,
        _audio_stream: &MovieAudioStream,
        _event_tx: &mpsc::Sender<DecoderEvent>,
        _frame_tx: &mpsc::SyncSender<DecodedFrame>,
    ) -> Result<(), String> {
        Err("movie decoding is currently available only on Windows".into())
    }
}

