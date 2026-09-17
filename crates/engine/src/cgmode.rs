use crate::cgdata::{CG_LIST_MODE0, CG_LIST_MODE1, CG_UNLOCK_PAIRS};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
pub const GRID_XS: [f32; 5] = [96.0, 272.0, 448.0, 624.0, 800.0];
pub const GRID_YS: [f32; 5] = [32.0, 144.0, 256.0, 368.0, 480.0];
pub const CELL_W: f32 = 140.0;
pub const CELL_H: f32 = 90.0;
pub const THUMB_W: f32 = 128.0;
pub const THUMB_H: f32 = 80.0;
pub const BTN_MODE: [f32; 4] = [96.0, 570.0, 448.0, 612.0];
pub const BTN_BACK: [f32; 4] = [600.0, 570.0, 720.0, 612.0];
pub const BTN_NEXT: [f32; 4] = [720.0, 570.0, 844.0, 612.0];
pub const BTN_RETURN: [f32; 4] = [844.0, 570.0, 946.0, 612.0];
pub fn cg_list(mode: usize) -> &'static [&'static str] {
    if mode == 0 { CG_LIST_MODE0 } else { CG_LIST_MODE1 }
}
#[derive(Default)]
pub struct CgSeen {
    pub seen: HashSet<String>,
}
impl CgSeen {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut seen = Self::default();
        for line in String::from_utf8_lossy(bytes).lines() {
            let name = line.trim();
            if !name.is_empty() {
                seen.insert(name);
            }
        }
        seen
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut names: Vec<&String> = self.seen.iter().collect();
        names.sort();
        names
            .into_iter()
            .flat_map(|name| name.bytes().chain(std::iter::once(b'\n')))
            .collect()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.seen.contains(&name.to_ascii_lowercase())
    }
    pub fn insert(&mut self, name: &str) {
        self.seen.insert(name.to_ascii_lowercase());
    }
    pub fn apply_pairs(&mut self) {
        for &(a, b) in CG_UNLOCK_PAIRS {
            if self.contains(a) && !self.contains(b) {
                self.insert(b);
            }
        }
    }
}
pub fn cgseen_path() -> PathBuf {
    crate::engine::Engine::save_dir().join("cgseen.tsv")
}
pub fn load_cgseen() -> CgSeen {
    std::fs::read(cgseen_path())
        .ok()
        .map_or_else(CgSeen::default, |bytes| CgSeen::from_bytes(&bytes))
}
pub fn persist_cgseen(seen: &CgSeen) {
    let p = cgseen_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&p, seen.to_bytes()) {
        log::warn!("CG 解锁集持久化失败 {}: {e:?}", p.display());
    }
}
pub struct CgGallery {
    pub mode: usize,
    pub page: usize,
    pub pages: usize,
    pub display: Vec<usize>,
    pub viewer: Option<CgViewer>,
    pub fade: u8,
    closing: bool,
    pub dead: bool,
}
impl CgGallery {
    pub fn open(seen: &Rc<RefCell<CgSeen>>) -> Self {
        {
            let mut s = seen.borrow_mut();
            s.apply_pairs();
        }
        persist_cgseen(&seen.borrow());
        let mut g = CgGallery {
            mode: 0,
            page: 0,
            pages: 0,
            display: Vec::new(),
            viewer: None,
            fade: 0,
            closing: false,
            dead: false,
        };
        g.rebuild();
        g
    }
    pub fn rebuild(&mut self) {
        let list = cg_list(self.mode);
        self.display = list
            .iter()
            .enumerate()
            .filter(|(_, n)| n.starts_with('*'))
            .map(|(i, _)| i)
            .collect();
        self.pages = self.display.len().div_ceil(25);
        self.page = self.page.min(self.pages.saturating_sub(1));
    }
    pub fn cell_entry(&self, cell: usize) -> Option<&'static str> {
        let abs = cell + 25 * self.page;
        let &idx = self.display.get(abs)?;
        cg_list(self.mode).get(idx).copied()
    }
    pub fn display_name(entry: &str) -> &str {
        entry.strip_prefix('*').unwrap_or(entry)
    }
    pub fn viewer_names(&self, cell: usize) -> Vec<String> {
        let Some(entry) = self.cell_entry(cell) else {
            return Vec::new();
        };
        let list = cg_list(self.mode);
        let start = 25 * self.page + cell;
        let Some(&idx) = self.display.get(start) else {
            return Vec::new();
        };
        let mut names = vec![Self::display_name(entry).to_string()];
        let mut i = idx + 1;
        while let Some(name) = list.get(i) {
            if name.starts_with('*') {
                break;
            }
            names.push((*name).to_string());
            i += 1;
        }
        names
    }
    pub fn command(&mut self, cmd: u8) {
        match cmd {
            2 => {
                self.page = if self.page == 0 {
                    self.pages.saturating_sub(1)
                } else {
                    self.page - 1
                };
            }
            3 => {
                self.page = (self.page + 1) % self.pages.max(1);
            }
            4 => self.close(),
            _ => {}
        }
    }
    pub fn close(&mut self) {
        self.closing = true;
    }
    pub fn tick(&mut self) {
        if self.closing {
            self.fade = self.fade.saturating_sub(10);
            if self.fade == 0 {
                self.dead = true;
            }
        } else {
            self.fade = (self.fade as u16 + 10).min(255) as u8;
        }
    }
    pub fn alpha(&self) -> f32 {
        self.fade as f32 / 255.0
    }
}
pub struct CgViewer {
    pub names: Vec<String>,
    pub idx: usize,
    pub fade: u16,
    pub closing: bool,
    pub win_x: i32,
    pub win_y: i32,
}
impl CgViewer {
    pub fn new(names: Vec<String>) -> Self {
        CgViewer {
            names,
            idx: 0,
            fade: 0,
            closing: false,
            win_x: 0,
            win_y: 0,
        }
    }
    pub fn current(&self) -> Option<&str> {
        self.names.get(self.idx).map(|s| s.as_str())
    }
    pub fn advance(&mut self) -> bool {
        if self.fade < 50 {
            self.fade = 50;
            return true;
        }
        if self.idx + 1 < self.names.len() {
            self.idx += 1;
            self.fade = 0;
            self.win_x = 0;
            self.win_y = 0;
            true
        } else {
            self.closing = true;
            false
        }
    }
    pub fn scroll(&mut self, dx: i32, dy: i32) {
        self.win_x += dx;
        self.win_y += dy;
    }
    pub fn tick(&mut self) -> bool {
        if self.closing {
            return self.fade > 0
                && {
                    self.fade = self.fade.saturating_sub(2);
                    self.fade == 0
                };
        }
        if self.fade < 50 {
            self.fade += 1;
        }
        false
    }
    pub fn alpha(&self) -> f32 {
        if self.closing { self.fade as f32 / 50.0 } else { self.fade as f32 / 50.0 }
    }
}

