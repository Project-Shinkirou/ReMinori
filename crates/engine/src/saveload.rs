use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;
use crate::storage::StorageBackend;
pub const SLOT_RECTS: [[f32; 4]; 10] = [
    [144.0, 48.0, 488.0, 147.0],
    [144.0, 154.0, 488.0, 252.0],
    [144.0, 260.0, 488.0, 358.0],
    [144.0, 366.0, 488.0, 464.0],
    [144.0, 472.0, 488.0, 570.0],
    [538.0, 48.0, 882.0, 147.0],
    [538.0, 154.0, 882.0, 252.0],
    [538.0, 260.0, 882.0, 358.0],
    [538.0, 366.0, 882.0, 464.0],
    [538.0, 472.0, 882.0, 570.0],
];
const NAV_RIGHT: [i32; 10] = [5, 6, 7, 8, 9, 1, 2, 3, 4, 0];
const NAV_LEFT: [i32; 10] = [9, 5, 6, 7, 8, 0, 1, 2, 3, 4];
pub fn tab_x(mode: u8) -> f32 {
    if mode == 0 { 422.0 } else { 380.0 }
}
fn nav_hit(x: f32, y: f32) -> u8 {
    if y >= 580.0 && y < 612.0 && x >= 532.0 {
        if x < 662.0 { 2 } else if x < 800.0 { 3 } else if x < 894.0 { 4 } else { 0 }
    } else if (112.0..342.0).contains(&x) && (555.0..600.0).contains(&y) {
        2
    } else {
        0
    }
}
pub fn hit_cmd(x: f32, y: f32) -> u8 {
    let col = if (143.0..487.0).contains(&x) {
        0
    } else if (537.0..881.0).contains(&x) {
        5
    } else {
        return nav_hit(x, y);
    };
    const ROWS: [f32; 5] = [27.0, 133.0, 239.0, 345.0, 451.0];
    for (r, &ry) in ROWS.iter().enumerate() {
        if (ry..ry + 99.0).contains(&y) {
            return 100 + (col + r) as u8;
        }
    }
    nav_hit(x, y)
}
pub fn slot_file(abs: usize, ext: &str) -> PathBuf {
    crate::engine::Engine::save_dir().join(format!("eden{abs:04}.{ext}"))
}
pub struct SaveSlot {
    pub exists: bool,
    pub timestamp: String,
    pub comment: String,
    pub mtime: Option<SystemTime>,
    pub thumb: PathBuf,
}
fn probe(abs: usize, storage: Option<&dyn StorageBackend>) -> SaveSlot {
    let sav = slot_file(abs, "sav");
    let thumb = slot_file(abs, "png");
    let mut s = SaveSlot {
        exists: false,
        timestamp: String::new(),
        comment: String::new(),
        mtime: None,
        thumb,
    };
    if let Some(storage) = storage {
        let sav_key = format!("saves/eden{abs:04}.sav");
        let thumb_key = format!("saves/eden{abs:04}.png");
        let Ok(Some(bytes)) = storage.read(&sav_key) else {
            return s;
        };
        if !matches!(storage.read(& thumb_key), Ok(Some(_))) {
            return s;
        }
        let Ok(file) = eden_formats::read_save_file(&bytes) else {
            return s;
        };
        s.exists = true;
        s.comment = file.header.comment;
        s.thumb = PathBuf::from(format!("~save/eden{abs:04}.png"));
        return s;
    }
    if !sav.is_file() || !s.thumb.is_file() {
        return s;
    }
    let Ok(meta) = std::fs::metadata(&sav) else {
        return s;
    };
    let Ok(bytes) = std::fs::read(&sav) else {
        return s;
    };
    let Ok(file) = eden_formats::read_save_file(&bytes) else {
        return s;
    };
    s.exists = true;
    s.comment = file.header.comment;
    if let Ok(m) = meta.modified() {
        s.mtime = Some(m);
        s.timestamp = crate::platform::timestamp_string(m);
    }
    s
}
pub struct SaveLoadMenu {
    pub mode: u8,
    pub from_game: bool,
    pub page: usize,
    pub slots: Vec<SaveSlot>,
    storage: Option<Rc<dyn StorageBackend>>,
    pub oldest: i32,
    pub newest: i32,
    pub selection: i32,
    pressed_cmd: u8,
    double_cmd: u8,
    state: u8,
    fade: u8,
    pub pending: Option<usize>,
    save_comment_request: Option<usize>,
    awaiting_save_comment: bool,
    pub pending_comment: Option<String>,
    pub dead: bool,
}
impl SaveLoadMenu {
    pub fn open(mode: u8, remembered_page: i32, from_game: bool) -> Self {
        Self::open_inner(mode, remembered_page, from_game, None)
    }
    pub fn open_with_storage(
        mode: u8,
        remembered_page: i32,
        from_game: bool,
        storage: Rc<dyn StorageBackend>,
    ) -> Self {
        Self::open_inner(mode, remembered_page, from_game, Some(storage))
    }
    fn open_inner(
        mode: u8,
        remembered_page: i32,
        from_game: bool,
        storage: Option<Rc<dyn StorageBackend>>,
    ) -> Self {
        let mut oldest = -1i32;
        let mut newest = -1i32;
        let (mut t_old, mut t_new) = (None, None);
        for abs in 0..100usize {
            let s = probe(abs, storage.as_deref());
            if !s.exists {
                continue;
            }
            let mt = s.mtime;
            if storage.is_some() && mt.is_none() {
                if oldest < 0 {
                    oldest = abs as i32;
                }
                newest = abs as i32;
                continue;
            }
            if t_old.map_or(true, |t| mt.map_or(false, |m| m < t)) && mt.is_some() {
                t_old = mt;
                oldest = abs as i32;
            }
            if t_new.map_or(true, |t| mt.map_or(false, |m| m >= t)) && mt.is_some() {
                t_new = mt;
                newest = abs as i32;
            }
        }
        let page = if mode == 0 {
            remembered_page.clamp(1, 9) as usize
        } else {
            (newest.max(0) as usize / 10).min(9)
        };
        let mut m = SaveLoadMenu {
            mode,
            from_game,
            page,
            slots: Vec::new(),
            storage,
            oldest,
            newest,
            selection: -1,
            pressed_cmd: 0,
            double_cmd: 0,
            state: 0,
            fade: 0,
            pending: None,
            save_comment_request: None,
            awaiting_save_comment: false,
            pending_comment: None,
            dead: false,
        };
        m.rebuild();
        m
    }
    pub fn rebuild(&mut self) {
        self.slots = (0..10)
            .map(|i| probe(self.page * 10 + i, self.storage.as_deref()))
            .collect();
    }
    pub fn turn_page(&mut self, dir: i32) {
        let min = if self.mode == 0 { 1 } else { 0 };
        let target = self.page as i32 + dir;
        if target >= min && target <= 9 {
            self.page = target as usize;
            self.selection = -1;
            self.rebuild();
        }
    }
    pub fn jump_page(&mut self, page: usize) {
        if self.page != page {
            self.page = page;
            self.selection = -1;
            self.rebuild();
        }
    }
    pub fn key_left(&mut self) {
        if self.selection < 0 {
            self.selection = 9;
        } else {
            self.selection = NAV_LEFT[self.selection as usize];
        }
    }
    pub fn key_right(&mut self) {
        if self.selection < 0 {
            self.selection = 0;
        } else {
            self.selection = NAV_RIGHT[self.selection as usize];
        }
    }
    pub fn key_up(&mut self) {
        if self.selection < 0 {
            self.selection = 9;
        } else if self.selection >= 5 {
            self.selection = if self.selection == 5 { 9 } else { self.selection - 1 };
        } else {
            self.selection = if self.selection == 0 { 4 } else { self.selection - 1 };
        }
    }
    pub fn key_down(&mut self) {
        if self.selection < 0 {
            self.selection = 0;
        } else if self.selection >= 5 {
            self.selection = if self.selection == 9 { 5 } else { self.selection + 1 };
        } else {
            self.selection = if self.selection == 4 { 0 } else { self.selection + 1 };
        }
    }
    pub fn pointer_press(&mut self, cmd: u8, click_count: u8) {
        self.pressed_cmd = cmd;
        if click_count >= 2 {
            self.double_cmd = cmd;
        }
    }
    pub fn pointer_release(&mut self, cmd: u8) {
        let pressed = std::mem::take(&mut self.pressed_cmd);
        if pressed != 0 && pressed == cmd {
            self.command(cmd);
        }
    }
    fn command(&mut self, cmd: u8) {
        match cmd {
            2 => self.turn_page(-1),
            3 => self.turn_page(1),
            4 => self.state = 2,
            c if (100..110).contains(&c) => {
                let i = (c - 100) as usize;
                let activate = self.selection == i as i32 && self.double_cmd == cmd;
                self.selection = i as i32;
                if activate && (self.mode == 0 || self.slots[i].exists) {
                    self.double_cmd = 0;
                    let abs = self.page * 10 + i;
                    if self.mode == 0 {
                        self.save_comment_request = Some(abs);
                        self.awaiting_save_comment = true;
                    } else {
                        self.pending = Some(abs);
                        self.state = 2;
                    }
                }
            }
            _ => {}
        }
    }
    pub fn confirm(&mut self) {
        if self.selection >= 0 {
            let c = 100 + self.selection as u8;
            self.double_cmd = c;
            self.command(c);
        }
    }
    pub fn tick(&mut self) {
        match self.state {
            0 => {
                self.fade = (self.fade as u16 + 4).min(255) as u8;
                if self.fade == 255 {
                    self.state = 1;
                }
            }
            2 => {
                self.fade = self.fade.saturating_sub(4);
                if self.fade == 0 {
                    self.dead = true;
                }
            }
            _ => {}
        }
    }
    pub fn close(&mut self) {
        if !self.awaiting_save_comment {
            self.state = 2;
        }
    }
    pub fn is_active(&self) -> bool {
        self.state == 1 && !self.awaiting_save_comment
    }
    pub fn take_save_comment_request(&mut self) -> Option<usize> {
        self.save_comment_request.take()
    }
    pub fn resolve_save_comment(&mut self, abs: usize, comment: Option<String>) -> bool {
        if !self.awaiting_save_comment || self.mode != 0 {
            return false;
        }
        self.awaiting_save_comment = false;
        if let Some(comment) = comment {
            self.pending = Some(abs);
            self.pending_comment = Some(comment);
            self.state = 2;
        }
        true
    }
    pub fn alpha(&self) -> f32 {
        self.fade as f32 / 255.0
    }
}

