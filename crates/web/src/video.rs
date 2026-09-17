use eden_engine::audio::AudioBackend;
use eden_engine::vfs::MovieSource;
use eden_engine::video::{
    MovieColorMatrix, MovieFrame, MoviePixels, MovieStatus, MovieTarget, VideoBackend,
};
use js_sys::Reflect;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Event, HtmlVideoElement};
const READY_STATE_HAVE_CURRENT_DATA: u16 = 2;
struct WebSession {
    video: HtmlVideoElement,
    url: String,
    target: MovieTarget,
    texture: Option<Arc<wgpu::Texture>>,
    texture_size: (u32, u32),
    last_uploaded_time: f64,
    current_frame: Option<MovieFrame>,
    blurred_paused: bool,
}
enum WebVideoState {
    Idle,
    Loading,
    Active(WebSession),
    FinishPending,
}
pub struct WebVideoBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    next_revision: Cell<u64>,
    state: Rc<RefCell<WebVideoState>>,
}
impl WebVideoBackend {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            next_revision: Cell::new(1),
            state: Rc::new(RefCell::new(WebVideoState::Idle)),
        }
    }
    pub fn set_blur_paused(&self, paused: bool) {
        let mut state = self.state.borrow_mut();
        let WebVideoState::Active(session) = &mut *state else {
            return;
        };
        if paused {
            if !session.video.paused() {
                let _ = session.video.pause();
                session.blurred_paused = true;
            }
        } else if session.blurred_paused {
            session.blurred_paused = false;
            if !session.video.ended() {
                if let Err(error) = session.video.play() {
                    log::warn!("恢复视频播放失败: {error:?}");
                }
            }
        }
    }
    fn teardown(session: WebSession) {
        let _ = session.video.pause();
        if let Some(parent) = session.video.parent_element() {
            let _ = parent.remove_child(&session.video);
        }
        if let Err(error) = web_sys::Url::revoke_object_url(&session.url) {
            log::warn!("释放视频 blob URL 失败: {error:?}");
        }
    }
    fn source_name(source: &MovieSource) -> String {
        match source {
            MovieSource::File(path) => {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("<invalid movie name>")
                    .to_owned()
            }
            MovieSource::Archive { name, .. } | MovieSource::Unavailable(name) => {
                name.clone()
            }
        }
    }
    fn sample_frame(&self, session: &mut WebSession) {
        if session.video.ready_state() < READY_STATE_HAVE_CURRENT_DATA {
            return;
        }
        let width = session.video.video_width();
        let height = session.video.video_height();
        if width == 0 || height == 0 {
            return;
        }
        if session.texture.is_none() {
            let texture = Arc::new(
                self
                    .device
                    .create_texture(
                        &wgpu::TextureDescriptor {
                            label: Some("web-movie"),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST
                                | wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        },
                    ),
            );
            session.texture = Some(texture);
            session.texture_size = (width, height);
        }
        let current_time = session.video.current_time();
        if current_time == session.last_uploaded_time {
            return;
        }
        let Some(texture) = session.texture.clone() else {
            return;
        };
        self.queue
            .copy_external_image_to_texture(
                &wgpu::CopyExternalImageSourceInfo {
                    source: wgpu::ExternalImageSource::HTMLVideoElement(
                        session.video.clone(),
                    ),
                    origin: wgpu::Origin2d::ZERO,
                    flip_y: false,
                },
                wgpu::CopyExternalImageDestInfo {
                    texture: texture.as_ref(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                    color_space: wgpu::PredefinedColorSpace::Srgb,
                    premultiplied_alpha: false,
                },
                wgpu::Extent3d {
                    width: session.texture_size.0,
                    height: session.texture_size.1,
                    depth_or_array_layers: 1,
                },
            );
        session.last_uploaded_time = current_time;
        let revision = self.next_revision.get().max(1);
        self.next_revision.set(revision.wrapping_add(1).max(1));
        let output_width = if session.target.width > 0 {
            session.target.width as f32
        } else {
            session.texture_size.0 as f32
        };
        let output_height = if session.target.height > 0 {
            session.target.height as f32
        } else {
            session.texture_size.1 as f32
        };
        session.current_frame = Some(MovieFrame {
            revision,
            width: session.texture_size.0,
            height: session.texture_size.1,
            pixels: MoviePixels::Rgba8Texture(texture),
            matrix: MovieColorMatrix::Bt601Limited,
            rect: [
                session.target.x as f32,
                session.target.y as f32,
                output_width,
                output_height,
            ],
        });
    }
}
fn install_loaded_session(
    state: &Rc<RefCell<WebVideoState>>,
    outcome: Result<WebSession, String>,
) {
    let mut slot = state.borrow_mut();
    if !matches!(&* slot, WebVideoState::Loading) {
        if let Ok(session) = outcome {
            WebVideoBackend::teardown(session);
        }
        return;
    }
    match outcome {
        Ok(session) => *slot = WebVideoState::Active(session),
        Err(message) => {
            log::warn!("{message}");
            *slot = WebVideoState::FinishPending;
        }
    }
}
fn spawn_load(state: Rc<RefCell<WebVideoState>>, name: String, target: MovieTarget) {
    spawn_local(async move {
        let skipped = |message: String| message;
        let Some(handle) = crate::browser_fs::movie_override_file(&name) else {
            install_loaded_session(
                &state,
                Err(
                    skipped(
                        format!(
                            "Web 视频缺少 override/mov/{name} 或同 stem 的 .mp4（原版 WMV3 \
                     浏览器不可解，先用 debug_workspace/transcode_movies.py 转码）；已跳过"
                        ),
                    ),
                ),
            );
            return;
        };
        let file = match JsFuture::from(handle.get_file()).await {
            Ok(value) => {
                match value.dyn_into::<web_sys::File>() {
                    Ok(file) => file,
                    Err(_) => {
                        install_loaded_session(
                            &state,
                            Err(
                                skipped(
                                    format!(
                                        "影片 {name} 的 getFile() 未返回 File；已跳过"
                                    ),
                                ),
                            ),
                        );
                        return;
                    }
                }
            }
            Err(error) => {
                install_loaded_session(
                    &state,
                    Err(
                        skipped(
                            format!(
                                "读取影片 {name} 失败: {}；已跳过",
                                js_error(error)
                            ),
                        ),
                    ),
                );
                return;
            }
        };
        let url = match web_sys::Url::create_object_url_with_blob(file.as_ref()) {
            Ok(url) => url,
            Err(error) => {
                install_loaded_session(
                    &state,
                    Err(
                        skipped(
                            format!(
                                "创建影片 {name} 的 blob URL 失败: {error:?}；已跳过"
                            ),
                        ),
                    ),
                );
                return;
            }
        };
        match mount_video_element(&name, &url, target).await {
            Ok(session) => {
                log::info!("Web 视频已装载 {name}");
                install_loaded_session(&state, Ok(session));
            }
            Err(error) => {
                let _ = web_sys::Url::revoke_object_url(&url);
                install_loaded_session(
                    &state,
                    Err(
                        skipped(
                            format!(
                                "装载视频元素 {name} 失败: {}; 已跳过",
                                js_error(error)
                            ),
                        ),
                    ),
                );
            }
        }
    });
}
async fn mount_video_element(
    name: &str,
    url: &str,
    target: MovieTarget,
) -> Result<WebSession, JsValue> {
    let window = web_sys::window()
        .ok_or_else(|| JsValue::from_str("window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let video: HtmlVideoElement = document
        .create_element("video")?
        .dyn_into::<HtmlVideoElement>()
        .map_err(|_| JsValue::from_str(
            "createElement(\"video\") 未返回 video 元素",
        ))?;
    video.set_attribute("style", "display:none")?;
    video.set_attribute("playsinline", "")?;
    video.set_attribute("preload", "auto")?;
    video.set_attribute("data-eden-movie", name)?;
    document
        .body()
        .ok_or_else(|| JsValue::from_str("document.body 不存在"))?
        .append_child(video.as_ref())?;
    video.set_src(url);
    match video.play() {
        Ok(promise) => {
            match JsFuture::from(promise).await {
                Ok(_) => {}
                Err(error) if is_not_allowed_error(&error) => {
                    log::warn!(
                        "视频自动播放被拦截，等待首次手势重试: {name}"
                    );
                    let retry_video = video.clone();
                    let retry = Closure::<
                        dyn FnMut(Event),
                    >::new(move |_event: Event| {
                        let _ = retry_video.play();
                    });
                    window
                        .add_event_listener_with_callback(
                            "pointerdown",
                            retry.as_ref().unchecked_ref(),
                        )?;
                    retry.forget();
                }
                Err(error) => {
                    log::warn!("视频 play() 被拒绝 {name}: {}", js_error(error));
                }
            }
        }
        Err(error) => {
            log::warn!("视频 play() 调用失败 {name}: {error:?}");
        }
    }
    Ok(WebSession {
        video,
        url: url.to_owned(),
        target,
        texture: None,
        texture_size: (0, 0),
        last_uploaded_time: f64::NAN,
        current_frame: None,
        blurred_paused: false,
    })
}
fn is_not_allowed_error(error: &JsValue) -> bool {
    Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotAllowedError")
}
fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            Reflect::get(&value, &JsValue::from_str("message")).ok()?.as_string()
        })
        .or_else(|| js_sys::JSON::stringify(&value).ok()?.as_string())
        .unwrap_or_else(|| "未知浏览器 API 错误".into())
}
impl VideoBackend for WebVideoBackend {
    fn play(
        &mut self,
        source: MovieSource,
        target: MovieTarget,
        _audio: &mut dyn AudioBackend,
    ) {
        {
            let mut state = self.state.borrow_mut();
            if let WebVideoState::Active(session) = std::mem::replace(
                &mut *state,
                WebVideoState::Loading,
            ) {
                Self::teardown(session);
            }
        }
        let name = Self::source_name(&source);
        spawn_load(Rc::clone(&self.state), name, target);
    }
    fn poll(&mut self, _audio: &mut dyn AudioBackend, volume: i64) -> MovieStatus {
        let mut state = self.state.borrow_mut();
        match &mut *state {
            WebVideoState::Idle => MovieStatus::Idle,
            WebVideoState::Loading => MovieStatus::Playing,
            WebVideoState::FinishPending => {
                *state = WebVideoState::Idle;
                MovieStatus::Finished
            }
            WebVideoState::Active(session) => {
                if let Some(error) = session.video.error() {
                    let code = error.code();
                    let src = session.video.src();
                    let active = std::mem::replace(
                        &mut *state,
                        WebVideoState::FinishPending,
                    );
                    if let WebVideoState::Active(session) = active {
                        Self::teardown(session);
                    }
                    log::error!("视频元素错误（code={code}）：{src}");
                    return MovieStatus::Finished;
                }
                session.video.set_volume((volume.clamp(0, 100) as f64) / 100.0);
                if session.video.ended() {
                    return MovieStatus::Finished;
                }
                self.sample_frame(session);
                MovieStatus::Playing
            }
        }
    }
    fn current_frame(&self) -> Option<MovieFrame> {
        match &*self.state.borrow() {
            WebVideoState::Active(session) => session.current_frame.clone(),
            _ => None,
        }
    }
    fn cleanup(&mut self, _audio: &mut dyn AudioBackend) {
        let mut state = self.state.borrow_mut();
        if let WebVideoState::Active(session) = std::mem::replace(
            &mut *state,
            WebVideoState::Idle,
        ) {
            Self::teardown(session);
        }
    }
}
