#[cfg(any(target_arch = "wasm32", test))]
fn canonical_component(name: &str) -> Result<String, String> {
    if name.is_empty() || name == "." || name == ".."
        || name.contains(['/', '\\', ':', '\0'])
    {
        return Err(format!("不安全的目录项名称: {name:?}"));
    }
    Ok(name.to_ascii_lowercase())
}
#[cfg(any(target_arch = "wasm32", test))]
fn canonical_override_key(parts: &[String]) -> Result<String, String> {
    if parts.is_empty() {
        return Err("override 路径为空".into());
    }
    parts
        .iter()
        .map(|part| canonical_component(part))
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}
#[cfg(any(target_arch = "wasm32", test))]
fn canonical_resource_name(name: &str) -> Result<String, String> {
    let normalized = name.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains([':', '\0']) {
            return Err(format!("不安全的资源名称: {name:?}"));
        }
        parts.push(part.to_ascii_lowercase());
    }
    if parts.is_empty() {
        return Err(format!("资源名称为空: {name:?}"));
    }
    Ok(parts.join("/"))
}
#[cfg(target_arch = "wasm32")]
mod platform {
    use super::{canonical_component, canonical_override_key, canonical_resource_name};
    use eden_engine::resource::ResourceRequestCache;
    use eden_engine::vfs::decode_png;
    use eden_engine::{
        Image, ResourceGroup, ResourceState, ResourceStore, StorageBackend,
    };
    use eden_formats::script::ScriptIr;
    use eden_formats::{decode, PazEntry, PazIndex, ResourceKind, PAZ_HEADER_LEN};
    use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        File, FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemHandle,
    };
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen::prelude::wasm_bindgen(js_name = edenHandleSave)]
        fn save_directory_handle(handle: &JsValue) -> Promise;
        #[wasm_bindgen::prelude::wasm_bindgen(js_name = edenHandleLoad)]
        fn load_directory_handle() -> Promise;
    }
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    const MAX_BOOT_FILE: u64 = 64 * 1024 * 1024;
    const MAX_RUNTIME_FILE: u64 = 512 * 1024 * 1024;
    const CPU_IMAGE_BUDGET: u64 = 256 * 1024 * 1024;
    const CPU_BYTE_BUDGET: u64 = 128 * 1024 * 1024;
    const REQUIRED_ARCHIVES: [(&str, usize); 7] = [
        ("scr.paz", 0),
        ("bg.paz", 1),
        ("st.paz", 3),
        ("sys.paz", 4),
        ("bgm.paz", 5),
        ("voice.paz", 6),
        ("se.paz", 7),
    ];
    thread_local! {
        static ACTIVE_MOUNT : RefCell < Option < BrowserDirectory >> = const {
        RefCell::new(None) };
    }
    #[derive(Clone)]
    pub struct BrowserFileSnapshot {
        file: File,
        pub name: String,
        pub size: u64,
        pub last_modified: f64,
    }
    impl BrowserFileSnapshot {
        async fn from_handle(handle: &FileSystemFileHandle) -> Result<Self, String> {
            let value = JsFuture::from(handle.get_file()).await.map_err(js_error)?;
            let file = value
                .dyn_into::<File>()
                .map_err(|_| "getFile() 未返回 File".to_string())?;
            let raw_size = file.size();
            if !raw_size.is_finite() || raw_size < 0.0
                || raw_size > MAX_SAFE_INTEGER as f64 || raw_size.fract() != 0.0
            {
                return Err(
                    format!(
                        "文件大小超出 JavaScript 安全整数范围: {raw_size}"
                    ),
                );
            }
            Ok(Self {
                name: file.name(),
                size: raw_size as u64,
                last_modified: file.last_modified(),
                file,
            })
        }
        pub async fn read_range(
            &self,
            offset: u64,
            length: usize,
        ) -> Result<Vec<u8>, String> {
            let length64 = u64::try_from(length)
                .map_err(|_| "读取长度溢出".to_string())?;
            let end = offset
                .checked_add(length64)
                .filter(|end| *end <= self.size && *end <= MAX_SAFE_INTEGER)
                .ok_or_else(|| {
                    format!(
                        "{} 范围越界: {offset:#x}+{length:#x}, size={:#x}", self
                        .name, self.size
                    )
                })?;
            let blob = self
                .file
                .slice_with_f64_and_f64(offset as f64, end as f64)
                .map_err(js_error)?;
            let buffer = JsFuture::from(blob.array_buffer()).await.map_err(js_error)?;
            let bytes = Uint8Array::new(&buffer);
            if bytes.length() as usize != length {
                return Err(
                    format!(
                        "{} 短读: expected {length}, got {}", self.name, bytes.length()
                    ),
                );
            }
            let mut output = vec![0; length];
            bytes.copy_to(&mut output);
            Ok(output)
        }
        async fn read_all_limited(&self) -> Result<Vec<u8>, String> {
            if self.size > MAX_BOOT_FILE {
                return Err(
                    format!(
                        "启动文件 {} 过大: {} bytes（上限 {}）", self.name,
                        self.size, MAX_BOOT_FILE
                    ),
                );
            }
            self.read_range(0, self.size as usize).await
        }
        async fn read_all_runtime(&self) -> Result<Vec<u8>, String> {
            if self.size > MAX_RUNTIME_FILE {
                return Err(
                    format!(
                        "资源文件 {} 过大: {} bytes（单次读取上限 {}）",
                        self.name, self.size, MAX_RUNTIME_FILE
                    ),
                );
            }
            let length = usize::try_from(self.size)
                .map_err(|_| {
                    format!("资源文件 {} 无法放入 WASM 内存", self.name)
                })?;
            self.read_range(0, length).await
        }
    }
    pub struct BrowserArchive {
        pub file: BrowserFileSnapshot,
        pub index: Rc<PazIndex>,
    }
    impl BrowserArchive {
        async fn open(
            handle: &FileSystemFileHandle,
            slot: usize,
            material: bool,
        ) -> Result<Self, String> {
            let file = BrowserFileSnapshot::from_handle(handle).await?;
            let header_bytes = file.read_range(0, PAZ_HEADER_LEN).await?;
            let header = PazIndex::parse_header(&header_bytes, 0xaa)
                .map_err(|error| format!("{} 归档头无效: {error}", file.name))?;
            let index_bytes = file
                .read_range(PAZ_HEADER_LEN as u64, header.index_size as usize)
                .await?;
            let index = PazIndex::parse_index(header, &index_bytes, slot, material)
                .map_err(|error| format!("{} 索引无效: {error}", file.name))?;
            Ok(Self {
                file,
                index: Rc::new(index),
            })
        }
        async fn read_entry(
            &self,
            entry: &eden_formats::PazEntry,
        ) -> Result<Vec<u8>, String> {
            let length = self
                .index
                .entry_read_len(entry, self.file.size)
                .map_err(|error| error.to_string())?;
            let mut encrypted = self.file.read_range(entry.offset64(), length).await?;
            self.index
                .decode_entry(entry, &mut encrypted)
                .map_err(|error| error.to_string())
        }
    }
    #[derive(Clone)]
    enum BrowserReadPlan {
        Override(FileSystemFileHandle),
        Archive { file: BrowserFileSnapshot, index: Rc<PazIndex>, entry: PazEntry },
    }
    impl BrowserReadPlan {
        async fn read(self) -> Result<Vec<u8>, String> {
            match self {
                Self::Override(handle) => {
                    BrowserFileSnapshot::from_handle(&handle)
                        .await?
                        .read_all_runtime()
                        .await
                }
                Self::Archive { file, index, entry } => {
                    let length = index
                        .entry_read_len(&entry, file.size)
                        .map_err(|error| error.to_string())?;
                    let mut encrypted = file.read_range(entry.offset64(), length).await?;
                    let mut bytes = index
                        .decode_entry(&entry, &mut encrypted)
                        .map_err(|error| error.to_string())?;
                    if let Some(kind) = ResourceKind::from_name(&entry.name) {
                        decode(&entry.name, entry.size, kind, &mut bytes);
                    }
                    Ok(bytes)
                }
            }
        }
    }
    struct BrowserImagePlan {
        primary: BrowserReadPlan,
        inherited_size: Option<BrowserReadPlan>,
    }
    impl BrowserImagePlan {
        async fn read(self) -> Result<Rc<Image>, String> {
            let bytes = self.primary.read().await?;
            let mut image = decode_png(&bytes).map_err(|error| error.to_string())?;
            if let Some(base) = self.inherited_size {
                match base.read().await {
                    Ok(bytes) => {
                        match decode_png(&bytes) {
                            Ok(base) => {
                                image.logical_w = base.logical_w;
                                image.logical_h = base.logical_h;
                            }
                            Err(error) => {
                                log::warn!("原版 PNG 逻辑尺寸解码失败: {error}")
                            }
                        }
                    }
                    Err(error) => {
                        log::warn!("原版 PNG 逻辑尺寸读取失败: {error}")
                    }
                }
            }
            Ok(Rc::new(image))
        }
    }
    pub struct BrowserDirectory {
        pub name: String,
        pub archives: HashMap<String, BrowserArchive>,
        pub override_files: HashMap<String, FileSystemFileHandle>,
        pub scripts: HashMap<String, Rc<ScriptIr>>,
        pub font: Option<(Vec<u8>, String)>,
        pub warnings: Vec<String>,
    }
    pub struct MountSummary {
        pub directory_name: String,
        pub archive_count: usize,
        pub archive_entries: usize,
        pub override_count: usize,
        pub script_count: usize,
        pub has_font: bool,
        pub warnings: Vec<String>,
    }
    pub enum RestoreResult {
        None,
        NeedsPermission(String),
    }
    struct CacheBudget {
        limit: u64,
        total: u64,
        clock: u64,
        entries: HashMap<String, (u64, u64)>,
    }
    impl CacheBudget {
        fn new(limit: u64) -> Self {
            Self {
                limit,
                total: 0,
                clock: 0,
                entries: HashMap::new(),
            }
        }
        fn touch(&mut self, key: &str) {
            self.clock = self.clock.wrapping_add(1);
            if let Some((_, used)) = self.entries.get_mut(key) {
                *used = self.clock;
            }
        }
        fn insert(&mut self, key: String, bytes: u64) -> Vec<String> {
            self.clock = self.clock.wrapping_add(1);
            if let Some((old, _)) = self.entries.insert(key.clone(), (bytes, self.clock))
            {
                self.total = self.total.saturating_sub(old);
            }
            self.total = self.total.saturating_add(bytes);
            let mut evicted = Vec::new();
            while self.total > self.limit {
                let candidate = self
                    .entries
                    .iter()
                    .filter(|(name, _)| name.as_str() != key)
                    .min_by_key(|(_, (_, used))| *used)
                    .map(|(name, _)| name.clone());
                let Some(candidate) = candidate else { break };
                if let Some((bytes, _)) = self.entries.remove(&candidate) {
                    self.total = self.total.saturating_sub(bytes);
                    evicted.push(candidate);
                }
            }
            evicted
        }
        fn remove(&mut self, key: &str) {
            if let Some((bytes, _)) = self.entries.remove(key) {
                self.total = self.total.saturating_sub(bytes);
            }
        }
    }
    pub struct BrowserResourceStore {
        bytes: Rc<ResourceRequestCache<(ResourceGroup, String), Rc<[u8]>>>,
        images: Rc<ResourceRequestCache<String, Rc<Image>>>,
        byte_budget: Rc<RefCell<CacheBudget>>,
        request_redraw: Rc<dyn Fn()>,
        storage: Rc<dyn StorageBackend>,
        image_budget: Rc<RefCell<CacheBudget>>,
    }
    impl BrowserResourceStore {
        pub fn new(
            request_redraw: Rc<dyn Fn()>,
            storage: Rc<dyn StorageBackend>,
        ) -> Self {
            Self {
                bytes: Rc::new(ResourceRequestCache::default()),
                images: Rc::new(ResourceRequestCache::default()),
                byte_budget: Rc::new(RefCell::new(CacheBudget::new(CPU_BYTE_BUDGET))),
                request_redraw,
                storage,
                image_budget: Rc::new(RefCell::new(CacheBudget::new(CPU_IMAGE_BUDGET))),
            }
        }
        #[allow(dead_code)]
        pub fn retry_bytes(
            &self,
            group: ResourceGroup,
            name: &str,
        ) -> Result<(), String> {
            let name = canonical_resource_name(name)?;
            self.bytes.retry(&(group, name.clone()));
            self.byte_budget.borrow_mut().remove(&format!("{}:{name}", group.name()));
            Ok(())
        }
        #[allow(dead_code)]
        pub fn retry_image(&self, name: &str) -> Result<(), String> {
            let name = canonical_resource_name(name)?;
            self.images.retry(&name);
            self.image_budget.borrow_mut().remove(&name);
            Ok(())
        }
    }
    impl ResourceStore for BrowserResourceStore {
        fn request_bytes(
            &self,
            group: ResourceGroup,
            name: &str,
        ) -> ResourceState<Rc<[u8]>> {
            let name = match canonical_resource_name(name) {
                Ok(name) => name,
                Err(error) => return ResourceState::Failed(error),
            };
            let key = (group, name.clone());
            let (start, current) = self.bytes.start_or_get(key.clone());
            if !start {
                if matches!(& current, ResourceState::Ready(_)) {
                    self.byte_budget
                        .borrow_mut()
                        .touch(&format!("{}:{name}", group.name()));
                }
                return current;
            }
            log::debug!("浏览器资源 Pending {}:{name}", group.name());
            let plan = ACTIVE_MOUNT
                .with(|active| {
                    active
                        .borrow()
                        .as_ref()
                        .ok_or_else(|| "尚未挂载游戏目录".to_string())
                        .map(|mount| mount.prepare_read(group, &name))
                });
            let Some(plan) = (match plan {
                Ok(plan) => plan,
                Err(error) => {
                    let state = ResourceState::Failed(error);
                    self.bytes.finish(key, state.clone());
                    return state;
                }
            }) else {
                log::warn!("浏览器资源 Missing {}:{name}", group.name());
                self.bytes.finish(key, ResourceState::Missing);
                return ResourceState::Missing;
            };
            let cache = Rc::clone(&self.bytes);
            let budget = Rc::clone(&self.byte_budget);
            let redraw = Rc::clone(&self.request_redraw);
            wasm_bindgen_futures::spawn_local(async move {
                let state: ResourceState<Rc<[u8]>> = match plan.read().await {
                    Ok(bytes) => ResourceState::Ready(Rc::from(bytes)),
                    Err(error) => {
                        log::error!(
                            "浏览器资源读取失败 {}:{}: {error}", group.name(),
                            name
                        );
                        ResourceState::Failed(error)
                    }
                };
                if let ResourceState::Ready(bytes) = &state {
                    let budget_key = format!("{}:{name}", group.name());
                    let evicted = budget
                        .borrow_mut()
                        .insert(budget_key, bytes.len() as u64);
                    for key in evicted {
                        if let Some((group, name)) = key.split_once(':') {
                            if let Some(group) = resource_group(group) {
                                cache.retry(&(group, name.to_owned()));
                            }
                        }
                    }
                }
                cache.finish(key, state);
                redraw();
            });
            ResourceState::Pending
        }
        fn request_image(&self, name: &str) -> ResourceState<Rc<Image>> {
            let name = match canonical_resource_name(name) {
                Ok(name) => name,
                Err(error) => return ResourceState::Failed(error),
            };
            let (start, current) = self.images.start_or_get(name.clone());
            if !start {
                if matches!(& current, ResourceState::Ready(_)) {
                    self.image_budget.borrow_mut().touch(&name);
                }
                return current;
            }
            log::debug!("浏览器图片 Pending {name}");
            if let Some(save_name) = name.strip_prefix("~save/") {
                let state = match self.storage.read(&format!("saves/{save_name}")) {
                    Ok(Some(bytes)) => {
                        match decode_png(&bytes) {
                            Ok(image) => ResourceState::Ready(Rc::new(image)),
                            Err(error) => ResourceState::Failed(error.to_string()),
                        }
                    }
                    Ok(None) => ResourceState::Missing,
                    Err(error) => ResourceState::Failed(error),
                };
                if let ResourceState::Ready(image) = &state {
                    let evicted = self
                        .image_budget
                        .borrow_mut()
                        .insert(name.clone(), image.rgba.len() as u64);
                    for key in evicted {
                        self.images.retry(&key);
                    }
                }
                self.images.finish(name, state.clone());
                return state;
            }
            let plan = ACTIVE_MOUNT
                .with(|active| {
                    active
                        .borrow()
                        .as_ref()
                        .ok_or_else(|| "尚未挂载游戏目录".to_string())
                        .map(|mount| mount.prepare_image(&name))
                });
            let Some(plan) = (match plan {
                Ok(plan) => plan,
                Err(error) => {
                    let state = ResourceState::Failed(error);
                    self.images.finish(name, state.clone());
                    return state;
                }
            }) else {
                log::warn!("浏览器图片 Missing {name}");
                self.images.finish(name, ResourceState::Missing);
                return ResourceState::Missing;
            };
            let cache = Rc::clone(&self.images);
            let budget = Rc::clone(&self.image_budget);
            let redraw = Rc::clone(&self.request_redraw);
            let cache_key = name.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let state = match plan.read().await {
                    Ok(image) => ResourceState::Ready(image),
                    Err(error) => {
                        log::error!(
                            "浏览器 PNG 读取或解码失败 {name}: {error}"
                        );
                        ResourceState::Failed(error)
                    }
                };
                if let ResourceState::Ready(image) = &state {
                    let evicted = budget
                        .borrow_mut()
                        .insert(cache_key.clone(), image.rgba.len() as u64);
                    for key in evicted {
                        cache.retry(&key);
                    }
                }
                cache.finish(cache_key, state);
                redraw();
            });
            ResourceState::Pending
        }
        fn script(&self, name: &str) -> Option<Rc<ScriptIr>> {
            let name = canonical_resource_name(name).ok()?;
            ACTIVE_MOUNT
                .with(|active| {
                    active
                        .borrow()
                        .as_ref()
                        .and_then(|mount| mount.scripts.get(&name).cloned())
                })
        }
        fn pump(&self) {}
        fn invalidate_image(&self, name: &str) {
            if let Ok(name) = canonical_resource_name(name) {
                self.images.retry(&name);
                self.image_budget.borrow_mut().remove(&name);
            }
        }
    }
    pub fn create_resource_store(
        request_redraw: Rc<dyn Fn()>,
        storage: Rc<dyn StorageBackend>,
    ) -> Result<Rc<dyn ResourceStore>, String> {
        let mounted = ACTIVE_MOUNT.with(|active| active.borrow().is_some());
        if !mounted {
            return Err("尚未挂载游戏目录".into());
        }
        Ok(Rc::new(BrowserResourceStore::new(request_redraw, storage)))
    }
    pub fn mounted_font() -> Result<(Vec<u8>, String), String> {
        ACTIVE_MOUNT
            .with(|active| {
                active
                    .borrow()
                    .as_ref()
                    .ok_or_else(|| "尚未挂载游戏目录".to_string())?
                    .font
                    .clone()
                    .ok_or_else(|| "挂载目录没有 Web 字体".to_string())
            })
    }
    pub fn movie_override_file(name: &str) -> Option<FileSystemFileHandle> {
        let canonical = canonical_resource_name(name).ok()?;
        let mut candidates = Vec::new();
        if let Some(stem) = canonical.strip_suffix(".avi") {
            candidates.push(format!("mov/{stem}.mp4"));
        }
        candidates.push(format!("mov/{canonical}"));
        ACTIVE_MOUNT
            .with(|active| {
                let mount = active.borrow();
                let mount = mount.as_ref()?;
                candidates.iter().find_map(|key| mount.override_files.get(key).cloned())
            })
    }
    impl BrowserDirectory {
        async fn mount(root: FileSystemDirectoryHandle) -> Result<Self, String> {
            let directory_name = root.name();
            let root_entries = enumerate(&root).await?;
            let mut files = HashMap::<String, FileSystemFileHandle>::new();
            let mut directories = HashMap::<String, FileSystemDirectoryHandle>::new();
            for (name, handle) in root_entries {
                let key = canonical_component(&name)?;
                if files.contains_key(&key) || directories.contains_key(&key) {
                    return Err(format!("根目录存在大小写折叠重名: {name}"));
                }
                match handle_kind(&handle)?.as_str() {
                    "file" => {
                        files.insert(key, handle.unchecked_into());
                    }
                    "directory" => {
                        directories.insert(key, handle.unchecked_into());
                    }
                    kind => return Err(format!("未知 FileSystemHandle kind: {kind}")),
                }
            }
            let missing: Vec<&str> = REQUIRED_ARCHIVES
                .iter()
                .map(|(name, _)| *name)
                .filter(|name| !files.contains_key(*name))
                .collect();
            if !missing.is_empty() {
                return Err(
                    format!("游戏目录缺少必需归档: {}", missing.join(", ")),
                );
            }
            let mut warnings = Vec::new();
            if !files.contains_key("mov.paz") {
                warnings.push("mov.paz 缺失；原版视频将被跳过".into());
            }
            let mut archives = HashMap::new();
            for (name, slot) in REQUIRED_ARCHIVES {
                let archive = BrowserArchive::open(files.get(name).unwrap(), slot, false)
                    .await?;
                archives.insert(name.trim_end_matches(".paz").to_owned(), archive);
            }
            if let Some(handle) = files.get("mov.paz") {
                let archive = BrowserArchive::open(handle, 9, true).await?;
                archives.insert("mov".into(), archive);
            }
            let snapshot_bytes = archives
                .values()
                .fold(
                    0_u64,
                    |total, archive| { total.saturating_add(archive.file.size) },
                );
            let newest_modified = archives
                .values()
                .map(|archive| archive.file.last_modified)
                .fold(0.0_f64, f64::max);
            warnings
                .push(
                    format!(
                        "归档快照已固定（合计 {snapshot_bytes} bytes，latest mtime={newest_modified:.0}）；运行中替换文件后请刷新或重新挂载"
                    ),
                );
            let override_files = if let Some(directory) = directories.get("override") {
                index_override(directory.clone()).await?
            } else {
                HashMap::new()
            };
            let mut mount = Self {
                name: directory_name,
                archives,
                override_files,
                scripts: HashMap::new(),
                font: None,
                warnings,
            };
            mount.load_scripts_and_font().await?;
            Ok(mount)
        }
        async fn load_scripts_and_font(&mut self) -> Result<(), String> {
            let archive = self
                .archives
                .get("scr")
                .ok_or_else(|| "scr.paz 未挂载".to_string())?;
            for entry in archive.index.entries.clone() {
                let mut bytes = archive.read_entry(&entry).await?;
                decode(&entry.name, entry.size, ResourceKind::Script, &mut bytes);
                self.scripts
                    .insert(
                        entry.name.to_ascii_lowercase(),
                        Rc::new(ScriptIr::parse_bytes(&bytes)),
                    );
            }
            let override_scripts: Vec<(String, FileSystemFileHandle)> = self
                .override_files
                .iter()
                .filter(|(key, _)| key.starts_with("scr/") && key.ends_with(".sc"))
                .map(|(key, handle)| (key[4..].to_owned(), handle.clone()))
                .collect();
            for (name, handle) in override_scripts {
                let snapshot = BrowserFileSnapshot::from_handle(&handle).await?;
                let bytes = snapshot.read_all_limited().await?;
                let text = String::from_utf8(bytes)
                    .map_err(|error| {
                        format!("override/scr/{name} 不是 UTF-8: {error}")
                    })?;
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                self.scripts
                    .insert(name.to_ascii_lowercase(), Rc::new(ScriptIr::parse(text)));
            }
            for key in ["fonts/default.ttf", "fonts/default.otf"] {
                if let Some(handle) = self.override_files.get(key) {
                    let snapshot = BrowserFileSnapshot::from_handle(handle).await?;
                    self.font = Some((
                        snapshot.read_all_limited().await?,
                        key.to_owned(),
                    ));
                    break;
                }
            }
            if self.font.is_none() {
                return Err(
                    "浏览器运行需要 override/fonts/default.ttf 或 default.otf"
                        .into(),
                );
            }
            Ok(())
        }
        fn archive_plan(
            &self,
            group: ResourceGroup,
            name: &str,
        ) -> Option<BrowserReadPlan> {
            let archive = self.archives.get(group.name())?;
            let entry = archive.index.find(name)?.clone();
            Some(BrowserReadPlan::Archive {
                file: archive.file.clone(),
                index: Rc::clone(&archive.index),
                entry,
            })
        }
        fn prepare_read(
            &self,
            group: ResourceGroup,
            name: &str,
        ) -> Option<BrowserReadPlan> {
            let override_key = format!("{}/{name}", group.name());
            self.override_files
                .get(&override_key)
                .cloned()
                .map(BrowserReadPlan::Override)
                .or_else(|| self.archive_plan(group, name))
        }
        fn prepare_image(&self, name: &str) -> Option<BrowserImagePlan> {
            for group in [ResourceGroup::Bg, ResourceGroup::St, ResourceGroup::Sys] {
                let override_key = format!("{}/{name}", group.name());
                if let Some(handle) = self.override_files.get(&override_key) {
                    return Some(BrowserImagePlan {
                        primary: BrowserReadPlan::Override(handle.clone()),
                        inherited_size: self.archive_plan(group, name),
                    });
                }
                if let Some(primary) = self.archive_plan(group, name) {
                    return Some(BrowserImagePlan {
                        primary,
                        inherited_size: None,
                    });
                }
            }
            None
        }
        fn summary(&self) -> MountSummary {
            MountSummary {
                directory_name: self.name.clone(),
                archive_count: self.archives.len(),
                archive_entries: self
                    .archives
                    .values()
                    .map(|archive| archive.index.entries.len())
                    .sum(),
                override_count: self.override_files.len(),
                script_count: self.scripts.len(),
                has_font: self.font.is_some(),
                warnings: self.warnings.clone(),
            }
        }
    }
    pub const PICKER_CANCELLED: &str = "__eden_picker_cancelled__";
    pub async fn select_and_mount() -> Result<MountSummary, String> {
        let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
        let picker = Reflect::get(
                window.as_ref(),
                &JsValue::from_str("showDirectoryPicker"),
            )
            .map_err(js_error)?
            .dyn_into::<Function>()
            .map_err(|_| "showDirectoryPicker unavailable".to_string())?;
        let options = Object::new();
        Reflect::set(
                &options,
                &JsValue::from_str("id"),
                &JsValue::from_str("eden-game-root"),
            )
            .map_err(js_error)?;
        Reflect::set(&options, &JsValue::from_str("mode"), &JsValue::from_str("read"))
            .map_err(js_error)?;
        let promise = picker
            .call1(window.as_ref(), &options)
            .map_err(js_error)?
            .dyn_into::<Promise>()
            .map_err(|_| "showDirectoryPicker 未返回 Promise".to_string())?;
        let root = JsFuture::from(promise)
            .await
            .map_err(|error| {
                if is_abort_error(&error) {
                    PICKER_CANCELLED.to_string()
                } else {
                    js_error(error)
                }
            })?
            .dyn_into::<FileSystemDirectoryHandle>()
            .map_err(|_| "目录选择器未返回目录句柄".to_string())?;
        finish_mount(root, true).await
    }
    pub async fn restore_saved_mount() -> Result<RestoreResult, String> {
        let value = JsFuture::from(load_directory_handle()).await.map_err(js_error)?;
        if value.is_null() || value.is_undefined() {
            return Ok(RestoreResult::None);
        }
        let root = value
            .dyn_into::<FileSystemDirectoryHandle>()
            .map_err(|_| "IndexedDB 中的游戏目录句柄无效".to_string())?;
        let name = root.name();
        Ok(RestoreResult::NeedsPermission(name))
    }
    async fn finish_mount(
        root: FileSystemDirectoryHandle,
        persist: bool,
    ) -> Result<MountSummary, String> {
        let persisted_handle = root.clone();
        let mut mount = BrowserDirectory::mount(root).await?;
        if persist {
            if let Err(error) = JsFuture::from(
                    save_directory_handle(persisted_handle.as_ref()),
                )
                .await
            {
                mount
                    .warnings
                    .push(
                        format!(
                            "目录句柄写入 IndexedDB 失败: {}", js_error(error)
                        ),
                    );
            }
        }
        let summary = mount.summary();
        ACTIVE_MOUNT.with(|active| *active.borrow_mut() = Some(mount));
        Ok(summary)
    }
    async fn index_override(
        root: FileSystemDirectoryHandle,
    ) -> Result<HashMap<String, FileSystemFileHandle>, String> {
        let mut files = HashMap::new();
        let mut pending = vec![(Vec::< String >::new(), root)];
        while let Some((prefix, directory)) = pending.pop() {
            for (name, handle) in enumerate(&directory).await? {
                canonical_component(&name)?;
                let mut parts = prefix.clone();
                parts.push(name.clone());
                match handle_kind(&handle)?.as_str() {
                    "directory" => pending.push((parts, handle.unchecked_into())),
                    "file" => {
                        let key = canonical_override_key(&parts)?;
                        if files.insert(key.clone(), handle.unchecked_into()).is_some() {
                            return Err(format!("override 大小写折叠重名: {key}"));
                        }
                    }
                    kind => return Err(format!("未知 FileSystemHandle kind: {kind}")),
                }
            }
        }
        Ok(files)
    }
    async fn enumerate(
        directory: &FileSystemDirectoryHandle,
    ) -> Result<Vec<(String, FileSystemHandle)>, String> {
        let iterator = directory.entries();
        let mut output = Vec::new();
        loop {
            let next = iterator.next().map_err(js_error)?;
            let item = JsFuture::from(next).await.map_err(js_error)?;
            if Reflect::get(&item, &JsValue::from_str("done"))
                .map_err(js_error)?
                .as_bool()
                .unwrap_or(false)
            {
                break;
            }
            let value = Reflect::get(&item, &JsValue::from_str("value"))
                .map_err(js_error)?;
            let pair = Array::from(&value);
            if pair.length() != 2 {
                return Err("目录迭代器返回了无效条目".into());
            }
            let name = pair
                .get(0)
                .as_string()
                .ok_or_else(|| "目录项名称不是字符串".to_string())?;
            let handle = pair
                .get(1)
                .dyn_into::<FileSystemHandle>()
                .map_err(|_| format!("目录项 {name} 没有 FileSystemHandle"))?;
            output.push((name, handle));
        }
        Ok(output)
    }
    fn handle_kind(handle: &FileSystemHandle) -> Result<String, String> {
        Reflect::get(handle.as_ref(), &JsValue::from_str("kind"))
            .map_err(js_error)?
            .as_string()
            .ok_or_else(|| "FileSystemHandle.kind 不是字符串".to_string())
    }
    fn resource_group(name: &str) -> Option<ResourceGroup> {
        Some(
            match name {
                "scr" => ResourceGroup::Scr,
                "bg" => ResourceGroup::Bg,
                "st" => ResourceGroup::St,
                "sys" => ResourceGroup::Sys,
                "bgm" => ResourceGroup::Bgm,
                "voice" => ResourceGroup::Voice,
                "se" => ResourceGroup::Se,
                "mov" => ResourceGroup::Mov,
                _ => return None,
            },
        )
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
    fn is_abort_error(value: &JsValue) -> bool {
        Reflect::get(value, &JsValue::from_str("name"))
            .ok()
            .and_then(|name| name.as_string())
            .is_some_and(|name| name == "AbortError")
    }
}
#[cfg(target_arch = "wasm32")]
pub use platform::{
    create_resource_store, mounted_font, movie_override_file, restore_saved_mount,
    select_and_mount, MountSummary, RestoreResult, PICKER_CANCELLED,
};

