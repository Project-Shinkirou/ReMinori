use eden_formats::script::ScriptIr;
use eden_formats::{decode, Ani, PazArchive, ResourceKind};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use eden_formats::profile::{ArchiveProfile, ProfileArchive};
#[derive(Debug)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub logical_w: f32,
    pub logical_h: f32,
    pub rgba: Vec<u8>,
}
impl Image {
    pub fn logical_size(&self) -> (f32, f32) {
        (self.logical_w, self.logical_h)
    }
    pub fn density(&self) -> (f32, f32) {
        (
            self.w as f32 / self.logical_w.max(1.0),
            self.h as f32 / self.logical_h.max(1.0),
        )
    }
}
const SLOT_NAMES: &[(&str, usize)] = &[
    ("scr", 0),
    ("bg", 1),
    ("st", 3),
    ("sys", 4),
    ("bgm", 5),
    ("voice", 6),
    ("se", 7),
];
#[derive(Debug)]
struct OverrideMount {
    files: HashMap<String, PathBuf>,
}
impl OverrideMount {
    fn open(root: PathBuf) -> anyhow::Result<Self> {
        if !root.is_dir() {
            anyhow::bail!("override 目录不存在: {}", root.display());
        }
        validate_override_config(&root)?;
        let mut files = HashMap::new();
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    log::warn!(
                        "忽略 override 符号链接: {}", entry.path().display()
                    );
                    continue;
                }
                if file_type.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let relative = entry.path().strip_prefix(&root)?.to_path_buf();
                let key = canonical_relative_path(&relative)?;
                if let Some(previous) = files.insert(key.clone(), entry.path()) {
                    anyhow::bail!(
                        "override 大小写折叠后重名 {key}: {} / {}", previous
                        .display(), relative.display()
                    );
                }
            }
        }
        log::info!("override: {}（{} 文件）", root.display(), files.len());
        Ok(OverrideMount { files })
    }
    fn path(&self, group: &str, name: &str) -> Option<&PathBuf> {
        let name = canonical_resource_name(name)?;
        self.files.get(&format!("{group}/{name}"))
    }
    fn path_by_key(&self, key: &str) -> Option<&PathBuf> {
        self.files.get(&key.to_ascii_lowercase())
    }
    fn read(&self, group: &str, name: &str) -> Option<Vec<u8>> {
        let path = self.path(group, name)?;
        fs::read(path)
            .map_err(|error| {
                log::warn!("读取 override {} 失败: {error}", path.display())
            })
            .ok()
    }
    fn contains(&self, group: &str, name: &str) -> bool {
        self.path(group, name).is_some()
    }
}
fn canonical_relative_path(path: &Path) -> anyhow::Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "override 路径不是 UTF-8: {}", path.display()
                        )
                    })?;
                if part.contains(':') || part.contains('\0') {
                    anyhow::bail!(
                        "override 路径包含非法字符: {}", path.display()
                    );
                }
                parts.push(part.to_ascii_lowercase());
            }
            _ => anyhow::bail!("override 路径非法: {}", path.display()),
        }
    }
    Ok(parts.join("/"))
}
fn canonical_resource_name(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let mut parts = Vec::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\0') {
            log::warn!("拒绝非法资源路径: {name:?}");
            return None;
        }
        parts.push(part.to_ascii_lowercase());
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}
fn validate_override_config(root: &Path) -> anyhow::Result<()> {
    let path = root.join("override.toml");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(());
    };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(['"', '\'']);
        match key {
            "format_version" if value != "1" => {
                anyhow::bail!("不支持的 override format_version={value}")
            }
            "script_encoding" if !value.eq_ignore_ascii_case("utf-8") => {
                anyhow::bail!("override 脚本目前只支持 UTF-8，不支持 {value}")
            }
            _ => {}
        }
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub enum MovieSource {
    File(PathBuf),
    Archive {
        archive: PathBuf, name: String, xor_key: u8,
        profile: Option<Arc<ArchiveProfile>>, cache_root: Option<PathBuf>,
    },
    Unavailable(String),
}
impl MovieSource {
    pub(crate) fn materialize(&self) -> Result<PathBuf, String> {
        match self {
            MovieSource::Unavailable(name) => {
                Err(format!("movie source unavailable on this platform: {name}"))
            }
            MovieSource::File(path) => Ok(path.clone()),
            MovieSource::Archive { archive, name, xor_key, profile, cache_root } => {
                let metadata = std::fs::metadata(archive)
                    .map_err(|error| format!("stat {}: {error}", archive.display()))?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos());
                let cache_root = cache_root.clone().unwrap_or_else(|| {
                    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("eden").join("movies")
                });
                std::fs::create_dir_all(&cache_root)
                    .map_err(|error| {
                        format!("create {}: {error}", cache_root.display())
                    })?;
                let safe_name = Path::new(name)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| format!("invalid movie name {name:?}"))?;
                let output = cache_root
                    .join(
                        format!(
                            "{}-{:x}-{:x}.{}", Path::new(safe_name).file_stem()
                            .and_then(| value | value.to_str()).unwrap_or("movie"),
                            metadata.len(), modified, Path::new(safe_name).extension()
                            .and_then(| value | value.to_str()).unwrap_or("avi")
                        ),
                    );
                if output.is_file() {
                    return Ok(output);
                }
                log::info!("解包影片 {safe_name} → {}", output.display());
                let bytes = if let Some(profile) = profile {
                    ProfileArchive::open(archive, "mov", Arc::clone(profile))
                        .and_then(|archive| archive.read(safe_name))
                        .map_err(|error| format!("read profile movie: {error}"))?
                } else {
                    PazArchive::open_main_with_material(archive, 9, *xor_key)
                        .and_then(|archive| archive.read(safe_name))
                        .map_err(|error| format!("read movie archive: {error}"))?
                };
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos());
                let temporary = output
                    .with_extension(
                        format!(
                            "{}.tmp-{}-{nonce}", output.extension().and_then(| value |
                            value.to_str()).unwrap_or("avi"), std::process::id()
                        ),
                    );
                std::fs::write(&temporary, &bytes)
                    .map_err(|error| format!("write {}: {error}", temporary.display()))?;
                if let Err(error) = std::fs::rename(&temporary, &output) {
                    if output.is_file() {
                        let _ = std::fs::remove_file(&temporary);
                    } else {
                        return Err(format!("publish {}: {error}", output.display()));
                    }
                }
                Ok(output)
            }
        }
    }
}
pub struct Vfs {
    root: PathBuf,
    override_mount: Option<OverrideMount>,
    xor_key: u8,
    archives: RefCell<HashMap<&'static str, Option<Rc<PazArchive>>>>,
    scripts: RefCell<HashMap<String, Option<Rc<ScriptIr>>>>,
    images: RefCell<HashMap<String, Option<Rc<Image>>>>,
    anis: RefCell<HashMap<String, Option<Rc<Ani>>>>,
    profile: Option<Arc<ArchiveProfile>>,
    profile_archives: HashMap<String, ProfileArchive>,
    movie_cache: Option<PathBuf>,
    unicode_saves: Cell<bool>,
}
impl Vfs {
    pub fn open(root: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Self::open_with_override(root, None)
    }
    pub fn open_with_override(
        root: impl Into<PathBuf>,
        override_root: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            anyhow::bail!("游戏目录不存在: {}", root.display());
        }
        let explicit_override = override_root.is_some();
        let override_root = override_root.unwrap_or_else(|| root.join("override"));
        let override_mount = if override_root.is_dir() {
            Some(OverrideMount::open(override_root)?)
        } else if explicit_override {
            anyhow::bail!("override 目录不存在: {}", override_root.display());
        } else {
            None
        };
        Ok(Vfs {
            root,
            override_mount,
            xor_key: 0xAA,
            archives: RefCell::new(HashMap::new()),
            scripts: RefCell::new(HashMap::new()),
            images: RefCell::new(HashMap::new()),
            anis: RefCell::new(HashMap::new()),
            profile: None,
            profile_archives: HashMap::new(),
            movie_cache: None,
            unicode_saves: Cell::new(false),
        })
    }
    pub fn open_profile(
        root: PathBuf, profile: Arc<ArchiveProfile>, movie_cache: PathBuf,
    ) -> anyhow::Result<Self> {
        let movie_cache = movie_cache.join(format!("{:08x}", profile.cache_tag()));
        let mut vfs = Self::open_with_override(root, None)?;
        for role in eden_formats::profile::ROLES {
            let archive = ProfileArchive::open(
                &vfs.root.join(format!("{role}.paz")), role, Arc::clone(&profile),
            )?;
            vfs.profile_archives.insert(role.into(), archive);
        }
        for replacement in profile.texture_overrides.values() {
            let path = vfs.root.join(replacement).canonicalize()?;
            anyhow::ensure!(path.starts_with(vfs.root.canonicalize()?), "texture override escapes game root");
        }
        vfs.profile = Some(profile);
        vfs.movie_cache = Some(movie_cache);
        Ok(vfs)
    }
    pub fn set_movie_cache(&mut self, root: PathBuf) {
        self.movie_cache = Some(root);
    }
    pub fn has_override(&self) -> bool {
        self.override_mount.is_some()
    }
    pub fn requires_unicode_saves(&self) -> bool {
        self.unicode_saves.get() || self.override_mount
            .as_ref()
            .is_some_and(|mount| mount.files.keys().any(|key| key.starts_with("scr/")))
    }
    fn override_path(&self, group: &str, name: &str) -> Option<&PathBuf> {
        self.override_mount.as_ref()?.path(group, name)
    }
    fn read_override(&self, group: &str, name: &str) -> Option<Vec<u8>> {
        self.override_mount.as_ref()?.read(group, name)
    }
    fn has_override_entry(&self, group: &str, name: &str) -> bool {
        self.override_mount.as_ref().is_some_and(|mount| mount.contains(group, name))
    }
    fn archive(&self, group: &'static str) -> Option<Rc<PazArchive>> {
        let mut arcs = self.archives.borrow_mut();
        if let Some(cached) = arcs.get(group) {
            return cached.clone();
        }
        let slot = SLOT_NAMES.iter().find(|(g, _)| *g == group).map(|(_, s)| *s)?;
        let path = self.root.join(format!("{group}.paz"));
        let opened = PazArchive::open_main(&path, slot, self.xor_key)
            .map_err(|e| {
                log::warn!("打开 {} 失败: {e}", path.display());
                e
            })
            .ok()
            .map(Rc::new);
        arcs.insert(group, opened.clone());
        opened
    }
    fn read_entry(&self, group: &'static str, name: &str) -> Option<Vec<u8>> {
        if let Some(archive) = self.profile_archives.get(group) {
            return archive.read(name).map_err(|error| {
                log::warn!("读取 profile PAZ {group} 失败: {error}");
            }).ok();
        }
        let arc = self.archive(group)?;
        let entry = arc.find(name)?;
        let entry_name = entry.name.clone();
        let mut data = arc
            .read(&entry_name)
            .map_err(|e| {
                log::warn!("读取 {group}:{entry_name} 失败: {e}");
                e
            })
            .ok()?;
        if let Some(kind) = ResourceKind::from_name(&entry_name) {
            decode(&entry_name, entry.size, kind, &mut data);
        }
        Some(data)
    }
    fn read_resource(&self, group: &'static str, name: &str) -> Option<Vec<u8>> {
        self.read_override(group, name).or_else(|| self.read_entry(group, name))
    }
    pub fn read_entry_debug(&self, group: &'static str, name: &str) -> Option<Vec<u8>> {
        self.read_resource(group, name)
    }
    pub(crate) fn movie_source(&self, name: &str) -> MovieSource {
        let safe_name = Path::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(name);
        let mp4_name = Path::new(safe_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|_| !safe_name.eq_ignore_ascii_case("mp4"))
            .map(|stem| format!("{stem}.mp4"));
        if let Some(mp4_name) = &mp4_name {
            if let Some(path) = self.override_path("mov", mp4_name) {
                if !mp4_name.eq_ignore_ascii_case(safe_name) {
                    log::info!(
                        "影片 override 用转码产物 {safe_name} → {}", path
                        .display()
                    );
                }
                return MovieSource::File(path.clone());
            }
        }
        if let Some(path) = self.override_path("mov", safe_name) {
            return MovieSource::File(path.clone());
        }
        let mut candidates = Vec::with_capacity(4);
        if let Some(mp4_name) = &mp4_name {
            candidates.push(self.root.join("movie").join(mp4_name));
            candidates.push(self.root.join(mp4_name));
        }
        candidates.push(self.root.join("movie").join(safe_name));
        candidates.push(self.root.join(safe_name));
        for candidate in candidates {
            if candidate.is_file() {
                return MovieSource::File(candidate);
            }
        }
        MovieSource::Archive {
            archive: self.root.join("mov.paz"),
            name: safe_name.to_ascii_lowercase(),
            xor_key: self.xor_key,
            profile: self.profile.clone(),
            cache_root: self.movie_cache.clone(),
        }
    }
    pub fn script(&self, name: &str) -> Option<Rc<ScriptIr>> {
        let key = canonical_resource_name(name)?;
        if let Some(Some(hit)) = self.scripts.borrow().get(&key) {
            return Some(Rc::clone(hit));
        }
        let ir = if let Some(bytes) = self.read_override("scr", &key) {
            match String::from_utf8(bytes) {
                Ok(text) => {
                    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                    Some(Rc::new(ScriptIr::parse(text)))
                }
                Err(error) => {
                    log::error!("override 脚本不是 UTF-8 {key}: {error}");
                    None
                }
            }
        } else {
            self.read_entry("scr", &key).and_then(|bytes| {
                match eden_formats::encoding::decode_script(&bytes) {
                    Ok((encoding, text)) => {
                        if encoding == eden_formats::encoding::ScriptEncoding::Gbk {
                            self.unicode_saves.set(true);
                        }
                        Some(Rc::new(ScriptIr::parse(&text)))
                    }
                    Err(error) => {
                        log::error!("native script decoding failed: {error}");
                        None
                    }
                }
            })
        };
        self.scripts.borrow_mut().insert(key, ir.clone());
        ir
    }
    pub fn image(&self, name: &str) -> Option<Rc<Image>> {
        if let Some(replacement) = self.profile.as_ref().and_then(|profile| {
            profile.texture_overrides.iter().find(|(source, _)| source.eq_ignore_ascii_case(name))
                .map(|(_, replacement)| replacement)
        }) {
            let key = format!("~profile:{}", name.to_ascii_lowercase());
            if let Some(image) = self.images.borrow().get(&key) {
                return image.clone();
            }
            let path = self.root.join(replacement);
            let image = std::fs::read(path).ok().and_then(|bytes| decode_png(&bytes).ok()).map(|mut image| {
                for group in ["bg", "st", "sys"] {
                    if let Some(base) = self.read_entry(group, name).and_then(|bytes| decode_png(&bytes).ok()) {
                        image.logical_w = base.logical_w;
                        image.logical_h = base.logical_h;
                        break;
                    }
                }
                Rc::new(image)
            });
            self.images.borrow_mut().insert(key, image.clone());
            return image;
        }
        let direct_path = Path::new(name);
        let direct_file = direct_path.is_file();
        let canonical = if name.starts_with("~anif:") {
            Some(name.to_ascii_lowercase())
        } else if direct_file && direct_path.is_absolute() {
            None
        } else {
            canonical_resource_name(name)
        };
        if canonical.is_none() && !direct_file {
            return None;
        }
        let key = canonical
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "~file:{}", direct_path.to_string_lossy().replace('\\', "/")
                    .to_ascii_lowercase()
                )
            });
        if let Some(hit) = self.images.borrow().get(&key) {
            return hit.clone();
        }
        let mut img: Option<Rc<Image>> = None;
        if let Some(resource_key) = canonical.as_deref() {
            for group in ["bg", "st", "sys"] {
                if let Some(bytes) = self.read_override(group, resource_key) {
                    img = decode_png(&bytes)
                        .map(|mut image| {
                            if let Some(base) = self
                                .read_entry(group, resource_key)
                                .and_then(|bytes| decode_png(&bytes).ok())
                            {
                                image.logical_w = base.logical_w;
                                image.logical_h = base.logical_h;
                            }
                            image
                        })
                        .map_err(|e| {
                            log::warn!(
                                "override PNG 解码失败 {group}/{resource_key}: {e}"
                            )
                        })
                        .ok()
                        .map(Rc::new);
                    break;
                }
                if let Some(bytes) = self.read_entry(group, resource_key) {
                    img = decode_png(&bytes)
                        .map_err(|e| log::warn!("PNG 解码失败 {resource_key}: {e}"))
                        .ok()
                        .map(Rc::new);
                    break;
                }
            }
        }
        if img.is_none() && direct_file {
            img = std::fs::read(name)
                .map_err(|e| {
                    log::warn!("读取 {name} 失败: {e}");
                    e
                })
                .ok()
                .and_then(|b| decode_png(&b).ok())
                .map(Rc::new);
        }
        self.images.borrow_mut().insert(key, img.clone());
        img
    }
    pub fn ani(&self, png_name: &str) -> Option<Rc<Ani>> {
        let key = canonical_resource_name(png_name)?;
        let ani_key = format!(
            "{}.ani", key.strip_suffix(".png").unwrap_or(key.as_str())
        );
        if let Some(hit) = self.anis.borrow().get(&ani_key) {
            return hit.clone();
        }
        if self.has_override_entry("bg", &key)
            && !self.has_override_entry("bg", &ani_key)
        {
            self.anis.borrow_mut().insert(ani_key, None);
            return None;
        }
        let ani = self
            .read_resource("bg", &ani_key)
            .and_then(|b| {
                Ani::parse(&b)
                    .map_err(|e| {
                        log::warn!("ani 解析失败 {ani_key}: {e}");
                    })
                    .ok()
            })
            .map(Rc::new);
        self.anis.borrow_mut().insert(ani_key, ani.clone());
        ani
    }
    pub fn store_image(&self, name: &str, img: Rc<Image>) {
        let key = name.to_ascii_lowercase();
        self.images.borrow_mut().insert(key, Some(img));
    }
    pub fn invalidate_image(&self, name: &str) {
        if let Some(key) = canonical_resource_name(name) {
            self.images.borrow_mut().remove(&key);
        }
        let path = Path::new(name);
        if path.is_absolute() {
            let key = format!(
                "~file:{}", path.to_string_lossy().replace('\\', "/")
                .to_ascii_lowercase()
            );
            self.images.borrow_mut().remove(&key);
        }
    }
    pub fn ani_frame_image(
        &self,
        png_name: &str,
        ani: &Ani,
        frame: usize,
    ) -> Option<(String, f32, f32, f32, f32)> {
        let f = ani.frames.get(frame)?;
        let (density_x, density_y) = self
            .image(png_name)
            .map(|image| image.density())
            .unwrap_or((1.0, 1.0));
        let key = format!(
            "~anif:{}#{}", png_name.strip_suffix(".png").unwrap_or(png_name)
            .to_ascii_lowercase(), frame
        );
        if self.images.borrow().get(&key).is_none() {
            let img = Rc::new(Image {
                w: f.w as u32,
                h: f.h as u32,
                logical_w: f.w as f32 / density_x.max(1e-6),
                logical_h: f.h as f32 / density_y.max(1e-6),
                rgba: f.rgba.clone(),
            });
            self.images.borrow_mut().insert(key.clone(), Some(img));
        }
        Some((
            key,
            f.x as f32 / density_x.max(1e-6),
            f.y as f32 / density_y.max(1e-6),
            f.w as f32 / density_x.max(1e-6),
            f.h as f32 / density_y.max(1e-6),
        ))
    }
    pub fn override_font(&self) -> Option<(Vec<u8>, String)> {
        let mount = self.override_mount.as_ref()?;
        for key in ["fonts/default.ttf", "fonts/default.otf"] {
            let Some(path) = mount.path_by_key(key) else {
                continue;
            };
            match fs::read(path) {
                Ok(bytes) => return Some((bytes, key.to_string())),
                Err(error) => {
                    log::warn!("读取补丁字体 {} 失败: {error}", path.display())
                }
            }
        }
        None
    }
}
pub fn decode_png(bytes: &[u8]) -> anyhow::Result<Image> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let n = (w as usize) * (h as usize);
            let mut out = Vec::with_capacity(n * 4);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let n = (w as usize) * (h as usize);
            let mut out = Vec::with_capacity(n * 4);
            for px in &buf[..n] {
                out.extend_from_slice(&[*px, *px, *px, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let n = (w as usize) * (h as usize);
            let mut out = Vec::with_capacity(n * 4);
            for px in buf.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        other => anyhow::bail!("不支持的颜色类型 {other:?}"),
    };
    Ok(Image {
        w,
        h,
        logical_w: w as f32,
        logical_h: h as f32,
        rgba,
    })
}
pub fn composite_ani(img: &mut Image, ani: &Ani, frame: usize) -> bool {
    let Some(f) = ani.frames.get(frame) else {
        return false;
    };
    let (x0, y0) = (f.x as i64, f.y as i64);
    let mut wrote = false;
    for row in 0..f.h as i64 {
        let dy = y0 + row;
        if dy < 0 || dy >= img.h as i64 {
            continue;
        }
        for col in 0..f.w as i64 {
            let dx = x0 + col;
            if dx < 0 || dx >= img.w as i64 {
                continue;
            }
            let src = ((row * f.w as i64 + col) * 4) as usize;
            let dst = ((dy * img.w as i64 + dx) * 4) as usize;
            let a = f.rgba[src + 3] as u32;
            if a == 0 {
                continue;
            }
            if a == 255 {
                img.rgba[dst..dst + 4].copy_from_slice(&f.rgba[src..src + 4]);
            } else {
                for i in 0..3 {
                    let s = f.rgba[src + i] as u32;
                    let d = img.rgba[dst + i] as u32;
                    img.rgba[dst + i] = ((s * a + d * (255 - a)) / 255) as u8;
                }
                img.rgba[dst + 3] = 255u8.max(img.rgba[dst + 3]);
            }
            wrote = true;
        }
    }
    wrote
}

