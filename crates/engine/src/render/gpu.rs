use crate::font::GlyphAtlas;
use crate::render::text::GlyphQuad;
use crate::render::Viewport;
use crate::resource::ResourceStore;
use crate::vfs::Image;
use crate::video::{MovieColorMatrix, MovieFrame, MoviePixels};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}
const SPRITE_WGSL: &str = r#"
struct VSOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VSOut {
    var o: VSOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.uv = uv; o.color = color;
    return o;
}
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;
@fragment fn fs(in: VSOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}
"#;
const GLYPH_WGSL: &str = r#"
struct VSOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VSOut {
    var o: VSOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.uv = uv; o.color = color;
    return o;
}
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;
@fragment fn fs(in: VSOut) -> @location(0) vec4<f32> {
    let a = textureSample(tex, samp, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
"#;
const MASK_WGSL: &str = r#"
struct VSOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VSOut {
    var o: VSOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.uv = uv; o.color = color;
    return o;
}
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex_new: texture_2d<f32>;
@group(0) @binding(2) var tex_mask: texture_2d<f32>;
@group(0) @binding(3) var prog: texture_2d<f32>; // 1x1 R32Float 进度
@fragment fn fs(in: VSOut) -> @location(0) vec4<f32> {
    let m = textureSample(tex_mask, samp, in.uv).r;
    let p = textureLoad(prog, vec2<i32>(0, 0), 0).r;
    let on = select(0.0, 1.0, m <= p);
    return textureSample(tex_new, samp, in.uv) * vec4<f32>(1.0, 1.0, 1.0, on);
}
"#;
const MOVIE_WGSL: &str = r#"
struct VSOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>) -> VSOut {
    var o: VSOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.uv = uv;
    return o;
}
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var luma_tex: texture_2d<f32>;
@group(0) @binding(2) var chroma_tex: texture_2d<f32>;

fn limited_yuv(uv_coord: vec2<f32>) -> vec3<f32> {
    let y = (textureSample(luma_tex, samp, uv_coord).r - 16.0 / 255.0)
        * (255.0 / 219.0);
    let uv = (textureSample(chroma_tex, samp, uv_coord).rg
        - vec2<f32>(128.0 / 255.0)) * (255.0 / 224.0);
    return vec3<f32>(y, uv.x, uv.y);
}

@fragment fn fs_601(in: VSOut) -> @location(0) vec4<f32> {
    let yuv = limited_yuv(in.uv);
    let rgb = vec3<f32>(
        yuv.x + 1.4020 * yuv.z,
        yuv.x - 0.344136 * yuv.y - 0.714136 * yuv.z,
        yuv.x + 1.7720 * yuv.y,
    );
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment fn fs_709(in: VSOut) -> @location(0) vec4<f32> {
    let yuv = limited_yuv(in.uv);
    let rgb = vec3<f32>(
        yuv.x + 1.5748 * yuv.z,
        yuv.x - 0.1873 * yuv.y - 0.4681 * yuv.z,
        yuv.x + 1.8556 * yuv.y,
    );
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;
struct TextureEntry {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    bytes: u64,
    last_used: u64,
}
struct EffectTexture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
}
enum MovieTextureKind {
    Nv12 { luma: wgpu::Texture, chroma: wgpu::Texture, bind: wgpu::BindGroup },
    Bgra8 { texture: wgpu::Texture, bind: wgpu::BindGroup },
    External { texture: Arc<wgpu::Texture>, bind: wgpu::BindGroup },
}
struct MovieTexture {
    revision: u64,
    width: u32,
    height: u32,
    kind: MovieTextureKind,
}
#[derive(Clone)]
pub struct FrameDraw {
    pub sprites: Vec<SpriteDraw>,
    pub glyphs: Vec<GlyphQuad>,
    pub sprites2: Vec<SpriteDraw>,
    pub glyphs2: Vec<GlyphQuad>,
    pub effects: Vec<EffectSpriteDraw>,
    pub(crate) movie: Option<MovieFrame>,
}
#[derive(Clone)]
pub struct SpriteDraw {
    pub image: String,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
}
impl SpriteDraw {
    pub fn full(image: impl Into<String>) -> Self {
        SpriteDraw {
            image: image.into(),
            rect: [0.0, 0.0, 1024.0, 640.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
        }
    }
}
#[derive(Clone)]
pub struct EffectSpriteDraw {
    pub image: String,
    pub x: i32,
    pub y: i32,
    pub alpha: u16,
}
fn max_effect_pixel(
    dst_value: &mut u32,
    dst_rgba: &mut [u8],
    src_rgba: &[u8],
    object_alpha: u32,
) {
    let src_alpha = src_rgba[3] as u32;
    let alpha = if object_alpha >= 255 {
        src_alpha
    } else {
        object_alpha * src_alpha / 255
    };
    if alpha == 0 {
        return;
    }
    let (r, g, b) = (src_rgba[0] as u32, src_rgba[1] as u32, src_rgba[2] as u32);
    let value = (alpha << 24) | (r << 16) | (g << 8) | b;
    if value <= *dst_value {
        return;
    }
    *dst_value = value;
    dst_rgba[0] = ((r * alpha + 127) / 255) as u8;
    dst_rgba[1] = ((g * alpha + 127) / 255) as u8;
    dst_rgba[2] = ((b * alpha + 127) / 255) as u8;
    dst_rgba[3] = alpha as u8;
}
#[derive(Clone)]
pub struct MaskTransitionDraw {
    pub image: String,
    pub mask: String,
    pub rect: [f32; 4],
    pub progress: f32,
}
struct CpuReadback {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    map_tx: std::sync::mpsc::SyncSender<Result<(), wgpu::BufferAsyncError>>,
    map_rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    sprite_pipe: wgpu::RenderPipeline,
    glyph_pipe: wgpu::RenderPipeline,
    mask_pipe: wgpu::RenderPipeline,
    movie_nv12_pipe_601: wgpu::RenderPipeline,
    movie_nv12_pipe_709: wgpu::RenderPipeline,
    movie_bgra_pipe: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    mask_bgl: wgpu::BindGroupLayout,
    movie_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    progress_tex: wgpu::Texture,
    progress_view: wgpu::TextureView,
    progress_bind: Option<wgpu::BindGroup>,
    effect_texture: Option<EffectTexture>,
    movie_texture: Option<MovieTexture>,
    textures: HashMap<String, TextureEntry>,
    texture_bytes: u64,
    texture_clock: u64,
    texture_budget: u64,
    atlas_texture: Option<wgpu::Texture>,
    atlas_bind: Option<wgpu::BindGroup>,
    atlas_uploaded_gen: u64,
    vertex_buffer: Option<wgpu::Buffer>,
    readback: Option<CpuReadback>,
}
impl Renderer {
    pub async fn new(
        window: impl wgpu::WindowHandle + 'static,
        phys_w: u32,
        phys_h: u32,
    ) -> anyhow::Result<Self> {
        let backends = std::env::var("EDEN_BACKEND")
            .ok()
            .and_then(|b| wgpu::Backends::from_bits(b.parse::<u32>().unwrap_or(0)))
            .unwrap_or(wgpu::Backends::PRIMARY);
        let instance = wgpu::Instance::new(
            &wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            },
        );
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window)))?;
        Self::initialize(instance, Some(surface), phys_w, phys_h).await
    }
    pub async fn new_offscreen(width: u32, height: u32) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        Self::initialize(instance, None, width, height).await
    }
    async fn initialize(
        instance: wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        phys_w: u32,
        phys_h: u32,
    ) -> anyhow::Result<Self> {
        let adapter = instance
            .request_adapter(
                &wgpu::RequestAdapterOptions {
                    power_preference: std::env::var_os("EDEN_GPU")
                        .map(|v| {
                            if v == "low" {
                                wgpu::PowerPreference::LowPower
                            } else {
                                wgpu::PowerPreference::HighPerformance
                            }
                        })
                        .unwrap_or(wgpu::PowerPreference::LowPower),
                    compatible_surface: surface.as_ref(),
                    force_fallback_adapter: false,
                },
            )
            .await?;
        if surface.is_none() {
            let info = adapter.get_info();
            anyhow::ensure!(info.device_type != wgpu::DeviceType::Cpu, "offscreen sessions require a hardware GPU");
            log::info!("ReMinori offscreen GPU: {} {:?} {:?}", info.name, info.device_type, info.backend);
        }
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("eden"),
                    ..Default::default()
                },
            )
            .await?;
        // Use the same native pipelines for both presentation and CPU capture.
        let caps = surface.as_ref().map(|surface| surface.get_capabilities(&adapter));
        let format = caps.as_ref().and_then(|caps| caps.formats.iter().copied().find(|f| {
            matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm)
        })).unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: phys_w.max(1),
            height: phys_h.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.as_ref().map_or(wgpu::CompositeAlphaMode::Auto, |caps| caps.alpha_modes[0]),
            view_formats: vec![],
        };
        if let Some(surface) = &surface {
            surface.configure(&device, &config);
        }
        let sampler = device
            .create_sampler(
                &wgpu::SamplerDescriptor {
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    address_mode_w: wgpu::AddressMode::ClampToEdge,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                },
            );
        let tex_bgl = device
            .create_bind_group_layout(
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some("tex-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(
                                wgpu::SamplerBindingType::Filtering,
                            ),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                },
            );
        let mask_bgl = device
            .create_bind_group_layout(
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some("mask-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(
                                wgpu::SamplerBindingType::Filtering,
                            ),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: false,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                },
            );
        let movie_bgl = device
            .create_bind_group_layout(
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some("movie-nv12-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(
                                wgpu::SamplerBindingType::Filtering,
                            ),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float {
                                    filterable: true,
                                },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                    ],
                },
            );
        let vb_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32x2, 2 => Float32x4
            ],
        };
        let make_pipe = |
            wgsl: &str,
            bgl: &wgpu::BindGroupLayout,
            vb_layout: &wgpu::VertexBufferLayout,
            blend: wgpu::BlendState,
            fragment_entry: &str,
        | -> wgpu::RenderPipeline {
            let module = device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("sprite-wgsl"),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                });
            let layout = device
                .create_pipeline_layout(
                    &wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &[bgl],
                        push_constant_ranges: &[],
                    },
                );
            device
                .create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some("pipe"),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[vb_layout.clone()],
                        },
                        primitive: wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleList,
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some(fragment_entry),
                            compilation_options: Default::default(),
                            targets: &[
                                Some(wgpu::ColorTargetState {
                                    format,
                                    blend: Some(blend),
                                    write_mask: wgpu::ColorWrites::ALL,
                                }),
                            ],
                        }),
                        multiview: None,
                        cache: None,
                    },
                )
        };
        let sprite_pipe = make_pipe(
            SPRITE_WGSL,
            &tex_bgl,
            &vb_layout,
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            "fs",
        );
        let glyph_pipe = make_pipe(
            GLYPH_WGSL,
            &tex_bgl,
            &vb_layout,
            wgpu::BlendState::ALPHA_BLENDING,
            "fs",
        );
        let mask_pipe = make_pipe(
            MASK_WGSL,
            &mask_bgl,
            &vb_layout,
            wgpu::BlendState::ALPHA_BLENDING,
            "fs",
        );
        let movie_nv12_pipe_601 = make_pipe(
            MOVIE_WGSL,
            &movie_bgl,
            &vb_layout,
            wgpu::BlendState::REPLACE,
            "fs_601",
        );
        let movie_nv12_pipe_709 = make_pipe(
            MOVIE_WGSL,
            &movie_bgl,
            &vb_layout,
            wgpu::BlendState::REPLACE,
            "fs_709",
        );
        let movie_bgra_pipe = make_pipe(
            SPRITE_WGSL,
            &tex_bgl,
            &vb_layout,
            wgpu::BlendState::REPLACE,
            "fs",
        );
        let progress_tex = device
            .create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("progress"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            );
        let progress_view = progress_tex.create_view(&Default::default());
        Ok(Renderer {
            device,
            queue,
            surface,
            config,
            sprite_pipe,
            glyph_pipe,
            mask_pipe,
            movie_nv12_pipe_601,
            movie_nv12_pipe_709,
            movie_bgra_pipe,
            tex_bgl,
            mask_bgl,
            movie_bgl,
            sampler,
            progress_tex,
            progress_view,
            progress_bind: None,
            effect_texture: None,
            movie_texture: None,
            textures: HashMap::new(),
            texture_bytes: 0,
            texture_clock: 0,
            texture_budget: 256 * 1024 * 1024,
            atlas_texture: None,
            atlas_bind: None,
            atlas_uploaded_gen: 0,
            vertex_buffer: None,
            readback: None,
        })
    }
    pub fn resize(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 && (self.config.width != w || self.config.height != h) {
            self.config.width = w;
            self.config.height = h;
            if let Some(surface) = &self.surface {
                surface.configure(&self.device, &self.config);
            }
            self.readback = None;
        }
    }
    pub fn config_width(&self) -> u32 {
        self.config.width
    }
    pub fn config_height(&self) -> u32 {
        self.config.height
    }
    fn ensure_texture(&mut self, name: &str, img: &Image, premultiply: bool) {
        self.texture_clock = self.texture_clock.wrapping_add(1);
        if let Some(entry) = self.textures.get_mut(name) {
            entry.last_used = self.texture_clock;
            return;
        }
        let size = wgpu::Extent3d {
            width: img.w,
            height: img.h,
            depth_or_array_layers: 1,
        };
        let texture = self
            .device
            .create_texture(
                &wgpu::TextureDescriptor {
                    label: Some(name),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            );
        let data: Vec<u8> = if premultiply {
            let mut px = img.rgba.clone();
            for p in px.chunks_exact_mut(4) {
                if p[3] != 255 {
                    let a = p[3] as u16;
                    for c in &mut p[..3] {
                        *c = ((*c as u16 * a + 127) / 255) as u8;
                    }
                }
            }
            px
        } else {
            img.rgba.clone()
        };
        self.queue
            .write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(img.w * 4),
                    rows_per_image: Some(img.h),
                },
                size,
            );
        let view = texture.create_view(&Default::default());
        let bind = self
            .device
            .create_bind_group(
                &wgpu::BindGroupDescriptor {
                    label: Some(name),
                    layout: &self.tex_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                    ],
                },
            );
        let bytes = u64::from(img.w) * u64::from(img.h) * 4;
        self.texture_bytes = self.texture_bytes.saturating_add(bytes);
        self.textures
            .insert(
                name.to_string(),
                TextureEntry {
                    _texture: texture,
                    view,
                    bind,
                    bytes,
                    last_used: self.texture_clock,
                },
            );
    }
    fn trim_texture_cache(&mut self, pinned: &std::collections::HashSet<String>) {
        while self.texture_bytes > self.texture_budget {
            let candidate = self
                .textures
                .iter()
                .filter(|(name, _)| !pinned.contains(*name))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(name, _)| name.clone());
            let Some(candidate) = candidate else { break };
            if let Some(entry) = self.textures.remove(&candidate) {
                self.texture_bytes = self.texture_bytes.saturating_sub(entry.bytes);
            }
        }
    }
    pub fn prefetch(&mut self, resources: &dyn ResourceStore, names: &[&str]) {
        for name in names {
            if name.is_empty() {
                continue;
            }
            if self.textures.contains_key(*name) {
                self.texture_clock = self.texture_clock.wrapping_add(1);
                if let Some(entry) = self.textures.get_mut(*name) {
                    entry.last_used = self.texture_clock;
                }
                continue;
            }
            if let Some(img) = resources.request_image(name).ready() {
                self.ensure_texture(name, &img, true);
            } else {
                log::warn!("贴图缺失: {name}");
            }
        }
    }
    fn update_effect_texture(
        &mut self,
        draws: &[EffectSpriteDraw],
        resources: &dyn ResourceStore,
    ) {
        const W: usize = 1024;
        const H: usize = 640;
        if self.effect_texture.is_none() {
            let texture = self
                .device
                .create_texture(
                    &wgpu::TextureDescriptor {
                        label: Some("effect-layer"),
                        size: wgpu::Extent3d {
                            width: W as u32,
                            height: H as u32,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                );
            let view = texture.create_view(&Default::default());
            let bind = self
                .device
                .create_bind_group(
                    &wgpu::BindGroupDescriptor {
                        label: Some("effect-layer-bind"),
                        layout: &self.tex_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                        ],
                    },
                );
            self.effect_texture = Some(EffectTexture { texture, bind });
        }
        let mut packed = vec![0u32; W * H];
        let mut rgba = vec![0u8; W * H * 4];
        for draw in draws {
            let Some(image) = resources.request_image(&draw.image).ready() else {
                continue;
            };
            let object_alpha = draw.alpha as u32;
            let logical_w = image.logical_w.max(1.0).round() as i32;
            let logical_h = image.logical_h.max(1.0).round() as i32;
            for ly in 0..logical_h {
                let dy = draw.y + ly;
                if !(0..H as i32).contains(&dy) {
                    continue;
                }
                let sy = ((ly as f32 + 0.5) * image.h as f32 / logical_h as f32)
                    .floor()
                    .clamp(0.0, image.h.saturating_sub(1) as f32) as usize;
                for lx in 0..logical_w {
                    let dx = draw.x + lx;
                    if !(0..W as i32).contains(&dx) {
                        continue;
                    }
                    let sx = ((lx as f32 + 0.5) * image.w as f32 / logical_w as f32)
                        .floor()
                        .clamp(0.0, image.w.saturating_sub(1) as f32) as usize;
                    let src = (sy * image.w as usize + sx) * 4;
                    let dst = dy as usize * W + dx as usize;
                    let out = dst * 4;
                    max_effect_pixel(
                        &mut packed[dst],
                        &mut rgba[out..out + 4],
                        &image.rgba[src..src + 4],
                        object_alpha,
                    );
                }
            }
        }
        self.queue
            .write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.effect_texture.as_ref().unwrap().texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((W * 4) as u32),
                    rows_per_image: Some(H as u32),
                },
                wgpu::Extent3d {
                    width: W as u32,
                    height: H as u32,
                    depth_or_array_layers: 1,
                },
            );
    }
    fn update_movie_texture(&mut self, frame: Option<&MovieFrame>) {
        let Some(frame) = frame else {
            return;
        };
        let compatible = self
            .movie_texture
            .as_ref()
            .is_some_and(|texture| {
                texture.width == frame.width && texture.height == frame.height
                    && match (&texture.kind, &frame.pixels) {
                        (MovieTextureKind::Nv12 { .. }, MoviePixels::Nv12 { .. })
                        | (MovieTextureKind::Bgra8 { .. }, MoviePixels::Bgra8(_)) => true,
                        (
                            MovieTextureKind::External { texture, .. },
                            MoviePixels::Rgba8Texture(source),
                        ) => Arc::ptr_eq(texture, source),
                        _ => false,
                    }
            });
        if !compatible {
            let size = wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            };
            let kind = match &frame.pixels {
                MoviePixels::Nv12 { .. } => {
                    let luma = self
                        .device
                        .create_texture(
                            &wgpu::TextureDescriptor {
                                label: Some("movie-luma"),
                                size,
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::R8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING
                                    | wgpu::TextureUsages::COPY_DST,
                                view_formats: &[],
                            },
                        );
                    let chroma = self
                        .device
                        .create_texture(
                            &wgpu::TextureDescriptor {
                                label: Some("movie-chroma"),
                                size: wgpu::Extent3d {
                                    width: frame.width / 2,
                                    height: frame.height / 2,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rg8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING
                                    | wgpu::TextureUsages::COPY_DST,
                                view_formats: &[],
                            },
                        );
                    let luma_view = luma.create_view(&Default::default());
                    let chroma_view = chroma.create_view(&Default::default());
                    let bind = self
                        .device
                        .create_bind_group(
                            &wgpu::BindGroupDescriptor {
                                label: Some("movie-nv12-bind"),
                                layout: &self.movie_bgl,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&luma_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&chroma_view),
                                    },
                                ],
                            },
                        );
                    MovieTextureKind::Nv12 {
                        luma,
                        chroma,
                        bind,
                    }
                }
                MoviePixels::Bgra8(_) => {
                    let texture = self
                        .device
                        .create_texture(
                            &wgpu::TextureDescriptor {
                                label: Some("movie-bgra"),
                                size,
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Bgra8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING
                                    | wgpu::TextureUsages::COPY_DST,
                                view_formats: &[],
                            },
                        );
                    let view = texture.create_view(&Default::default());
                    let bind = self
                        .device
                        .create_bind_group(
                            &wgpu::BindGroupDescriptor {
                                label: Some("movie-bgra-bind"),
                                layout: &self.tex_bgl,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&view),
                                    },
                                ],
                            },
                        );
                    MovieTextureKind::Bgra8 {
                        texture,
                        bind,
                    }
                }
                MoviePixels::Rgba8Texture(external) => {
                    let view = external.create_view(&Default::default());
                    let bind = self
                        .device
                        .create_bind_group(
                            &wgpu::BindGroupDescriptor {
                                label: Some("movie-external-bind"),
                                layout: &self.tex_bgl,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&view),
                                    },
                                ],
                            },
                        );
                    MovieTextureKind::External {
                        texture: Arc::clone(external),
                        bind,
                    }
                }
            };
            self.movie_texture = Some(MovieTexture {
                revision: 0,
                width: frame.width,
                height: frame.height,
                kind,
            });
        }
        let texture = self.movie_texture.as_mut().unwrap();
        if texture.revision == frame.revision {
            return;
        }
        match (&texture.kind, &frame.pixels) {
            (
                MovieTextureKind::Nv12 { luma, chroma, .. },
                MoviePixels::Nv12 { luma: y, chroma: uv },
            ) => {
                self.queue
                    .write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: luma,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        y,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(frame.width),
                            rows_per_image: Some(frame.height),
                        },
                        wgpu::Extent3d {
                            width: frame.width,
                            height: frame.height,
                            depth_or_array_layers: 1,
                        },
                    );
                self.queue
                    .write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: chroma,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        uv,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(frame.width),
                            rows_per_image: Some(frame.height / 2),
                        },
                        wgpu::Extent3d {
                            width: frame.width / 2,
                            height: frame.height / 2,
                            depth_or_array_layers: 1,
                        },
                    );
            }
            (MovieTextureKind::Bgra8 { texture, .. }, MoviePixels::Bgra8(pixels)) => {
                self.queue
                    .write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(frame.width * 4),
                            rows_per_image: Some(frame.height),
                        },
                        wgpu::Extent3d {
                            width: frame.width,
                            height: frame.height,
                            depth_or_array_layers: 1,
                        },
                    );
            }
            (MovieTextureKind::External { .. }, MoviePixels::Rgba8Texture(_)) => {}
            _ => unreachable!(),
        }
        texture.revision = frame.revision;
    }
    fn ensure_atlas(&mut self, atlas: &mut GlyphAtlas) -> bool {
        let dims = atlas.atlas_dims();
        if self.atlas_texture.is_none() {
            let texture = self
                .device
                .create_texture(
                    &wgpu::TextureDescriptor {
                        label: Some("glyph-atlas"),
                        size: wgpu::Extent3d {
                            width: dims.0,
                            height: dims.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::R8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                );
            let view = texture.create_view(&Default::default());
            let bind = self
                .device
                .create_bind_group(
                    &wgpu::BindGroupDescriptor {
                        label: Some("glyph-bind"),
                        layout: &self.tex_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                        ],
                    },
                );
            self.atlas_texture = Some(texture);
            self.atlas_bind = Some(bind);
            atlas.dirty = true;
        }
        if atlas.dirty {
            let size = wgpu::Extent3d {
                width: dims.0,
                height: dims.1,
                depth_or_array_layers: 1,
            };
            self.queue
                .write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: self.atlas_texture.as_ref().unwrap(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    atlas.atlas_data(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(dims.0),
                        rows_per_image: Some(dims.1),
                    },
                    size,
                );
            atlas.dirty = false;
            self.atlas_uploaded_gen += 1;
            true
        } else {
            false
        }
    }
    fn quad_verts(
        vp: &Viewport,
        rect: [f32; 4],
        uv: [f32; 4],
        color: [f32; 4],
    ) -> [Vertex; 6] {
        let [x0, y0, x1, y1] = vp.rect_to_ndc(rect[0], rect[1], rect[2], rect[3]);
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[0] + uv[2], uv[1] + uv[3]);
        [
            Vertex {
                pos: [x0, y0],
                uv: [u0, v0],
                color,
            },
            Vertex {
                pos: [x1, y0],
                uv: [u1, v0],
                color,
            },
            Vertex {
                pos: [x0, y1],
                uv: [u0, v1],
                color,
            },
            Vertex {
                pos: [x0, y1],
                uv: [u0, v1],
                color,
            },
            Vertex {
                pos: [x1, y0],
                uv: [u1, v0],
                color,
            },
            Vertex {
                pos: [x1, y1],
                uv: [u1, v1],
                color,
            },
        ]
    }
    pub fn render(
        &mut self,
        vp: &Viewport,
        draw: &FrameDraw,
        mask_tr: Option<&MaskTransitionDraw>,
        atlas: &mut GlyphAtlas,
        resources: &dyn ResourceStore,
    ) -> anyhow::Result<()> {
        if self.surface.is_none() {
            self.ensure_readback(vp);
            let view = self.readback.as_ref().unwrap().view.clone();
            self.submit_to(vp, draw, mask_tr, atlas, resources, &view);
            self.device.poll(wgpu::PollType::Wait)?;
            return Ok(());
        }
        let surface = self.surface.as_ref().expect("surface checked above");
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                log::warn!("获取表面失败: {e}（重配置后跳过本帧）");
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        self.submit_to(vp, draw, mask_tr, atlas, resources, &view);
        frame.present();
        Ok(())
    }
    pub fn render_clear(&mut self, rgba: [f64; 4]) -> anyhow::Result<()> {
        let surface = self.surface.as_ref().ok_or_else(|| anyhow::anyhow!("no presentation surface"))?;
        let frame = match surface.get_current_texture() {
            Ok(frame) => frame,
            Err(error) => {
                log::warn!("获取表面失败: {error}（重配置后跳过本帧）");
                surface.configure(&self.device, &self.config);
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(
                &wgpu::CommandEncoderDescriptor {
                    label: Some("eden-web-smoke-clear"),
                },
            );
        {
            let _pass = encoder
                .begin_render_pass(
                    &wgpu::RenderPassDescriptor {
                        label: Some("eden-web-smoke-pass"),
                        color_attachments: &[
                            Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                        r: rgba[0],
                                        g: rgba[1],
                                        b: rgba[2],
                                        a: rgba[3],
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                        ],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    },
                );
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
    pub fn is_offscreen(&self) -> bool { self.surface.is_none() }
    fn ensure_readback(&mut self, vp: &Viewport) {
        let width = vp.phys_w;
        let height = vp.phys_h;
        if self.readback.as_ref().is_some_and(|target| target.width == width && target.height == height) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shinki-final-frame"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shinki-final-readback"),
            size: u64::from(width.div_ceil(64) * 256) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let (map_tx, map_rx) = std::sync::mpsc::sync_channel(1);
        self.readback = Some(CpuReadback { texture, view, buffer, width, height, map_tx, map_rx });
    }
    pub fn render_cpu_into(
        &mut self,
        vp: &Viewport,
        draw: &FrameDraw,
        mask_tr: Option<&MaskTransitionDraw>,
        atlas: &mut GlyphAtlas,
        resources: &dyn ResourceStore,
        pixels: &mut [u8],
    ) -> anyhow::Result<()> {
        let width = vp.phys_w;
        let height = vp.phys_h;
        anyhow::ensure!(pixels.len() == width as usize * height as usize * 4, "CPU frame slot has incorrect size");
        let bpr = width.div_ceil(64) * 256;
        self.ensure_readback(vp);
        let view = self.readback.as_ref().unwrap().view.clone();
        self.submit_to(vp, draw, mask_tr, atlas, resources, &view);
        let target = self.readback.as_ref().unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let tx = target.map_tx.clone();
        target.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| { let _ = tx.send(result); });
        self.device.poll(wgpu::PollType::Wait)?;
        target.map_rx.recv()??;
        let mapped = target.buffer.slice(..).get_mapped_range();
        for (output, row) in pixels.chunks_exact_mut(width as usize * 4).zip(mapped.chunks_exact(bpr as usize)) {
            output.copy_from_slice(&row[..output.len()]);
        }
        drop(mapped);
        target.buffer.unmap();
        Ok(())
    }
    pub fn render_offscreen(
        &mut self,
        vp: &Viewport,
        draw: &FrameDraw,
        mask_tr: Option<&MaskTransitionDraw>,
        atlas: &mut GlyphAtlas,
        resources: &dyn ResourceStore,
        out: &Path,
    ) -> anyhow::Result<()> {
        let size = wgpu::Extent3d {
            width: vp.phys_w,
            height: vp.phys_h,
            depth_or_array_layers: 1,
        };
        let texture = self
            .device
            .create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("offscreen"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            );
        let view = texture.create_view(&Default::default());
        self.submit_to(vp, draw, mask_tr, atlas, resources, &view);
        let bpr = ((vp.phys_w * 4 + 255) / 256 * 256) as u64;
        let buf = self
            .device
            .create_buffer(
                &wgpu::BufferDescriptor {
                    label: Some("shot"),
                    size: bpr * vp.phys_h as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
            );
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr as u32),
                    rows_per_image: Some(vp.phys_h),
                },
            },
            size,
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice
            .map_async(
                wgpu::MapMode::Read,
                move |r| {
                    let _ = tx.send(r);
                },
            );
        self.device.poll(wgpu::PollType::Wait)?;
        rx.recv()??;
        let data = slice.get_mapped_range().to_vec();
        let mut rgba = Vec::with_capacity((vp.phys_w * vp.phys_h * 4) as usize);
        for row in data.chunks_exact(bpr as usize) {
            for px in row[..(vp.phys_w * 4) as usize].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, vp.phys_w, vp.phys_h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header()?;
            writer.write_image_data(&rgba)?;
        }
        std::fs::write(out, png_bytes)?;
        log::info!("离屏截图 → {}", out.display());
        Ok(())
    }
    pub fn render_thumbnail_png(
        &mut self,
        draw: &FrameDraw,
        mask_tr: Option<&MaskTransitionDraw>,
        atlas: &mut GlyphAtlas,
        resources: &dyn ResourceStore,
    ) -> anyhow::Result<Vec<u8>> {
        const SW: u32 = 1024;
        const SH: u32 = 640;
        const TW: u32 = 128;
        const TH: u32 = 80;
        let vp = Viewport::new(SW, SH);
        let size = wgpu::Extent3d {
            width: SW,
            height: SH,
            depth_or_array_layers: 1,
        };
        let texture = self
            .device
            .create_texture(
                &wgpu::TextureDescriptor {
                    label: Some("thumb"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            );
        let view = texture.create_view(&Default::default());
        self.submit_to(&vp, draw, mask_tr, atlas, resources, &view);
        let buf = self
            .device
            .create_buffer(
                &wgpu::BufferDescriptor {
                    label: Some("thumb-buf"),
                    size: (SW * SH * 4) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
            );
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SW * 4),
                    rows_per_image: Some(SH),
                },
            },
            size,
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice
            .map_async(
                wgpu::MapMode::Read,
                move |r| {
                    let _ = tx.send(r);
                },
            );
        self.device.poll(wgpu::PollType::Wait)?;
        rx.recv()??;
        let bgra = slice.get_mapped_range().to_vec();
        let (fx, fy) = (SW / TW, SH / TH);
        let mut rgba = vec![0u8; (TW * TH * 4) as usize];
        for ty in 0..TH {
            for tx in 0..TW {
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for y in 0..fy {
                    let row_base = ((ty * fy + y) * SW + tx * fx) as usize * 4;
                    for x in 0..fx {
                        let p = row_base + (x as usize) * 4;
                        b += bgra[p] as u32;
                        g += bgra[p + 1] as u32;
                        r += bgra[p + 2] as u32;
                    }
                }
                let n = (fx * fy) as u32;
                let o = ((ty * TW + tx) as usize) * 4;
                rgba[o] = (r / n) as u8;
                rgba[o + 1] = (g / n) as u8;
                rgba[o + 2] = (b / n) as u8;
                rgba[o + 3] = 255;
            }
        }
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, TW, TH);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header()?;
            writer.write_image_data(&rgba)?;
        }
        Ok(png_bytes)
    }
    fn submit_to(
        &mut self,
        vp: &Viewport,
        draw: &FrameDraw,
        mask_tr: Option<&MaskTransitionDraw>,
        atlas: &mut GlyphAtlas,
        resources: &dyn ResourceStore,
        view: &wgpu::TextureView,
    ) {
        self.update_movie_texture(draw.movie.as_ref());
        let mut verts: Vec<Vertex> = Vec::with_capacity(
            draw.sprites.len() * 6 + draw.glyphs.len() * 6 + draw.sprites2.len() * 6
                + draw.glyphs2.len() * 6 + usize::from(draw.movie.is_some()) * 6 + 12,
        );
        let mut sprite_ranges: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        let mut base_sprite_count = 0usize;
        for s in &draw.sprites {
            let start = verts.len();
            verts.extend_from_slice(&Self::quad_verts(vp, s.rect, s.uv, s.color));
            sprite_ranges.push((start..verts.len(), s.image.clone()));
            base_sprite_count += 1;
        }
        let glyph_start = verts.len();
        let glyph_end = glyph_start + draw.glyphs.len() * 6;
        let dims = atlas.atlas_dims();
        for g in &draw.glyphs {
            let uv = [
                g.atlas[0] as f32 / dims.0 as f32,
                g.atlas[1] as f32 / dims.1 as f32,
                g.atlas[2] as f32 / dims.0 as f32,
                g.atlas[3] as f32 / dims.1 as f32,
            ];
            verts.extend_from_slice(&Self::quad_verts(vp, g.rect, uv, g.color));
        }
        let mask_range = mask_tr
            .map(|m| {
                let start = verts.len();
                verts
                    .extend_from_slice(
                        &Self::quad_verts(vp, m.rect, [0.0, 0.0, 1.0, 1.0], [1.0; 4]),
                    );
                start..verts.len()
            });
        let effect_range = if draw.effects.is_empty() {
            None
        } else {
            self.update_effect_texture(&draw.effects, resources);
            let start = verts.len();
            verts
                .extend_from_slice(
                    &Self::quad_verts(
                        vp,
                        [0.0, 0.0, 1024.0, 640.0],
                        [0.0, 0.0, 1.0, 1.0],
                        [1.0; 4],
                    ),
                );
            Some(start..verts.len())
        };
        for s in &draw.sprites2 {
            let start = verts.len();
            verts.extend_from_slice(&Self::quad_verts(vp, s.rect, s.uv, s.color));
            sprite_ranges.push((start..verts.len(), s.image.clone()));
        }
        let glyph2_start = verts.len();
        for g in &draw.glyphs2 {
            let uv = [
                g.atlas[0] as f32 / dims.0 as f32,
                g.atlas[1] as f32 / dims.1 as f32,
                g.atlas[2] as f32 / dims.0 as f32,
                g.atlas[3] as f32 / dims.1 as f32,
            ];
            verts.extend_from_slice(&Self::quad_verts(vp, g.rect, uv, g.color));
        }
        let glyph2_end = verts.len();
        let movie_range = draw
            .movie
            .as_ref()
            .map(|movie| {
                let start = verts.len();
                verts
                    .extend_from_slice(
                        &Self::quad_verts(vp, movie.rect, [0.0, 0.0, 1.0, 1.0], [1.0; 4]),
                    );
                start..verts.len()
            });
        let mut pinned: std::collections::HashSet<String> = sprite_ranges
            .iter()
            .map(|(_, name)| name.clone())
            .collect();
        if let Some(mask) = mask_tr {
            pinned.insert(mask.image.clone());
            pinned.insert(mask.mask.clone());
        }
        for name in &pinned {
            self.texture_clock = self.texture_clock.wrapping_add(1);
            if let Some(entry) = self.textures.get_mut(name) {
                entry.last_used = self.texture_clock;
            }
        }
        let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        let buffer = self
            .vertex_buffer
            .take()
            .filter(|b| b.size() >= (verts.len() * std::mem::size_of::<Vertex>()) as u64)
            .unwrap_or_else(|| {
                self.device
                    .create_buffer(
                        &wgpu::BufferDescriptor {
                            label: Some("verts"),
                            size: ((verts.len() + 256) * std::mem::size_of::<Vertex>())
                                as u64,
                            usage,
                            mapped_at_creation: false,
                        },
                    )
            });
        self.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&verts));
        if let Some(m) = mask_tr {
            self.queue
                .write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.progress_tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::cast_slice(&[m.progress.clamp(0.0, 1.0) + 1.0 / 255.0]),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
        }
        if let Some(m) = mask_tr {
            if !self.textures.contains_key(&m.image) {
                if let Some(img) = resources.request_image(&m.image).ready() {
                    self.ensure_texture(&m.image, &img, true);
                }
            }
            if !self.textures.contains_key(&m.mask) {
                if let Some(img) = resources.request_image(&m.mask).ready() {
                    self.ensure_texture(&m.mask, &img, false);
                }
            }
            let new_view = self.textures.get(&m.image).map(|e| e.view.clone());
            let mask_view = self.textures.get(&m.mask).map(|e| e.view.clone());
            if let (Some(nv), Some(mv)) = (new_view, mask_view) {
                self.progress_bind = Some(
                    self
                        .device
                        .create_bind_group(
                            &wgpu::BindGroupDescriptor {
                                label: Some("mask-bind"),
                                layout: &self.mask_bgl,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(&nv),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::TextureView(&mv),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::TextureView(
                                            &self.progress_view,
                                        ),
                                    },
                                ],
                            },
                        ),
                );
            }
        }
        self.ensure_atlas(atlas);
        self.vertex_buffer = Some(buffer);
        let mut enc = self
            .device
            .create_command_encoder(
                &wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                },
            );
        {
            let mut rpass = enc
                .begin_render_pass(
                    &wgpu::RenderPassDescriptor {
                        label: Some("main"),
                        color_attachments: &[
                            Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            }),
                        ],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    },
                );
            let (sx, sy, sw, sh) = vp.scissor();
            rpass.set_scissor_rect(sx, sy, sw.max(1), sh.max(1));
            rpass.set_vertex_buffer(0, self.vertex_buffer.as_ref().unwrap().slice(..));
            rpass.set_pipeline(&self.sprite_pipe);
            for (range, name) in sprite_ranges.iter().take(base_sprite_count) {
                if let Some(t) = self.textures.get(name) {
                    rpass.set_bind_group(0, &t.bind, &[]);
                    rpass.draw(range.start as u32..range.end as u32, 0..1);
                }
            }
            if let (Some(range), Some(bind)) = (
                mask_range,
                self.progress_bind.as_ref(),
            ) {
                rpass.set_pipeline(&self.mask_pipe);
                rpass.set_bind_group(0, bind, &[]);
                rpass.draw(range.start as u32..range.end as u32, 0..1);
            }
            if let (Some(range), Some(effect)) = (
                effect_range,
                self.effect_texture.as_ref(),
            ) {
                rpass.set_pipeline(&self.sprite_pipe);
                rpass.set_bind_group(0, &effect.bind, &[]);
                rpass.draw(range.start as u32..range.end as u32, 0..1);
            }
            if !draw.glyphs.is_empty() {
                if let Some(bind) = &self.atlas_bind {
                    rpass.set_pipeline(&self.glyph_pipe);
                    rpass.set_bind_group(0, bind, &[]);
                    rpass.draw(glyph_start as u32..glyph_end as u32, 0..1);
                }
            }
            if base_sprite_count < sprite_ranges.len() {
                rpass.set_pipeline(&self.sprite_pipe);
                for (range, name) in sprite_ranges.iter().skip(base_sprite_count) {
                    if let Some(t) = self.textures.get(name) {
                        rpass.set_bind_group(0, &t.bind, &[]);
                        rpass.draw(range.start as u32..range.end as u32, 0..1);
                    }
                }
            }
            if !draw.glyphs2.is_empty() {
                if let Some(bind) = &self.atlas_bind {
                    rpass.set_pipeline(&self.glyph_pipe);
                    rpass.set_bind_group(0, bind, &[]);
                    rpass.draw(glyph2_start as u32..glyph2_end as u32, 0..1);
                }
            }
            if let (Some(range), Some(movie), Some(texture)) = (
                movie_range,
                draw.movie.as_ref(),
                self.movie_texture.as_ref(),
            ) {
                match &texture.kind {
                    MovieTextureKind::Nv12 { bind, .. } => {
                        rpass
                            .set_pipeline(
                                match movie.matrix {
                                    MovieColorMatrix::Bt601Limited => &self.movie_nv12_pipe_601,
                                    MovieColorMatrix::Bt709Limited => &self.movie_nv12_pipe_709,
                                },
                            );
                        rpass.set_bind_group(0, bind, &[]);
                    }
                    MovieTextureKind::Bgra8 { bind, .. }
                    | MovieTextureKind::External { bind, .. } => {
                        rpass.set_pipeline(&self.movie_bgra_pipe);
                        rpass.set_bind_group(0, bind, &[]);
                    }
                }
                rpass.draw(range.start as u32..range.end as u32, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        self.trim_texture_cache(&pinned);
    }
}

