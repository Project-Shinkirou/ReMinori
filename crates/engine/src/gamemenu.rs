pub const BACK_XY: (f32, f32) = (846.0, 456.0);
pub const BACK_WH: (f32, f32) = (172.0, 169.0);
pub const BUTTONS: [(f32, f32, f32, f32); 5] = [
    (869.0, 497.0, 52.0, 39.0),
    (946.0, 478.0, 50.0, 51.0),
    (958.0, 553.0, 36.0, 27.0),
    (918.0, 583.0, 33.0, 34.0),
    (873.0, 557.0, 37.0, 37.0),
];
pub const BTN_NAMES: [&str; 5] = [
    "gameMenuSkip.png",
    "gameMenuQSave.png",
    "gameMenuSystem.png",
    "gameMenuLoad.png",
    "gameMenuSave.png",
];
pub const SAVE_GUARDED: [usize; 3] = [1, 3, 4];
pub const ZONE: [f32; 4] = [858.0, 465.0, 1024.0, 640.0];
pub const CORNER: [f32; 4] = [984.0, 600.0, 1024.0, 640.0];
const OPEN_PX: i32 = 10;
const CLOSE_PX: i32 = 20;
const OPEN_ALPHA: i32 = 200;
const BTN_DIM: u8 = 165;
const BTN_LIT: u8 = 255;
const BTN_STEP: u8 = 2;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
    Collapsed,
    Opening,
    Open,
    Closing,
}
#[derive(Debug, PartialEq)]
pub enum MouseOutcome {
    None,
    Consumed,
    Command(usize),
}
#[derive(Debug, Clone)]
pub struct GameMenu {
    pub state: MenuState,
    anim_t: u32,
    btn_alpha: [u8; 5],
    btn_hover: [bool; 5],
    held: bool,
    pressed_cmd: Option<usize>,
    pub corner_pause: bool,
    pub adv_stop: bool,
    pub last_qsave_pc: Option<u32>,
}
impl Default for GameMenu {
    fn default() -> Self {
        Self {
            state: MenuState::Collapsed,
            anim_t: 0,
            btn_alpha: [0; 5],
            btn_hover: [false; 5],
            held: false,
            pressed_cmd: None,
            corner_pause: false,
            adv_stop: false,
            last_qsave_pc: None,
        }
    }
}
fn in_rect(x: f32, y: f32, r: [f32; 4]) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}
pub fn button_hit(x: f32, y: f32) -> Option<usize> {
    BUTTONS
        .iter()
        .position(|&(bx, by, bw, bh)| x >= bx && x < bx + bw && y >= by && y < by + bh)
}
impl GameMenu {
    pub fn cancel_pointer(&mut self) {
        self.held = false;
        self.pressed_cmd = None;
        self.btn_hover.fill(false);
    }
    pub fn pointer_left(&mut self) {
        self.cancel_pointer();
        if !matches!(self.state, MenuState::Collapsed) {
            self.state = MenuState::Closing;
            self.anim_t = 0;
        }
    }
    pub fn visible(&self) -> bool {
        !matches!(self.state, MenuState::Collapsed)
    }
    pub fn back_draw(&self) -> Option<([f32; 4], f32)> {
        let (w, h) = (BACK_WH.0 as i32, BACK_WH.1 as i32);
        let (pw, ph, alpha) = match self.state {
            MenuState::Collapsed => return None,
            MenuState::Opening => {
                let pw = (OPEN_PX * self.anim_t as i32).min(w);
                let ph = (OPEN_PX * self.anim_t as i32).min(h);
                let a = OPEN_ALPHA * pw / w;
                (pw, ph, a.max(1))
            }
            MenuState::Open => (w, h, 255),
            MenuState::Closing => {
                let pw = (w - CLOSE_PX * self.anim_t as i32).max(1);
                let ph = (h - CLOSE_PX * self.anim_t as i32).max(1);
                (pw, ph, 255)
            }
        };
        let x = (1018 - pw) as f32;
        let y = (625 - ph) as f32;
        Some(([x, y, pw as f32, ph as f32], alpha as f32 / 255.0))
    }
    pub fn button_draw(&self, i: usize) -> Option<([f32; 4], f32)> {
        if self.state != MenuState::Open {
            return None;
        }
        let (bx, by, bw, bh) = BUTTONS[i];
        let rect = if self.btn_hover[i] {
            [bx - bw / 4.0, by - bh / 4.0, bw * 1.5, bh * 1.5]
        } else {
            [bx, by, bw, bh]
        };
        Some((rect, self.btn_alpha[i] as f32 / 255.0))
    }
    fn begin_open(&mut self) {
        self.state = MenuState::Opening;
        self.anim_t = 0;
    }
    fn begin_close(&mut self) {
        if self.state != MenuState::Collapsed {
            self.state = MenuState::Closing;
            self.anim_t = 0;
            self.btn_hover = [false; 5];
        }
    }
    pub fn hide_now(&mut self) {
        self.state = MenuState::Collapsed;
        self.btn_hover = [false; 5];
        self.btn_alpha = [0; 5];
    }
    pub fn toggle(&mut self) {
        match self.state {
            MenuState::Collapsed => {
                self.begin_open();
                self.corner_pause = false;
            }
            _ => self.begin_close(),
        }
    }
    pub fn tick(
        &mut self,
        allowed: bool,
        cursor: Option<(f32, f32)>,
        prohibit_save: bool,
    ) {
        if !allowed && matches!(self.state, MenuState::Open | MenuState::Opening) {
            self.begin_close();
        }
        match self.state {
            MenuState::Opening => {
                self.anim_t += 1;
                if OPEN_PX * self.anim_t as i32 >= BACK_WH.0 as i32
                    && OPEN_PX * self.anim_t as i32 >= BACK_WH.1 as i32
                {
                    self.state = MenuState::Open;
                }
            }
            MenuState::Closing => {
                self.anim_t += 1;
                if BACK_WH.0 as i32 - CLOSE_PX * self.anim_t as i32 <= 1
                    && BACK_WH.1 as i32 - CLOSE_PX * self.anim_t as i32 <= 1
                {
                    self.hide_now();
                }
            }
            MenuState::Open => {
                if let Some((x, y)) = cursor {
                    for i in 0..5 {
                        let hit = button_hit(x, y) == Some(i);
                        self.btn_hover[i] = hit
                            && !(prohibit_save && SAVE_GUARDED.contains(&i));
                    }
                }
                for i in 0..5 {
                    let target = if self.btn_hover[i] { BTN_LIT } else { BTN_DIM };
                    let a = self.btn_alpha[i];
                    self.btn_alpha[i] = if a < target {
                        a.saturating_add(BTN_STEP).min(target)
                    } else {
                        a.saturating_sub(BTN_STEP).max(target)
                    };
                }
            }
            MenuState::Collapsed => {}
        }
    }
    pub fn mouse(
        &mut self,
        x: f32,
        y: f32,
        pressed: bool,
        allowed: bool,
        prohibit_save: bool,
    ) -> MouseOutcome {
        let corner = in_rect(x, y, CORNER);
        match self.state {
            MenuState::Collapsed => {
                if corner {
                    self.adv_stop = true;
                    if allowed {
                        self.begin_open();
                        self.corner_pause = false;
                    } else {
                        self.corner_pause = true;
                    }
                }
                MouseOutcome::None
            }
            MenuState::Open => {
                let hit = button_hit(x, y);
                for i in 0..5 {
                    let h = hit == Some(i);
                    self.btn_hover[i] = h
                        && !(prohibit_save && SAVE_GUARDED.contains(&i));
                }
                if let Some(_i) = hit {
                    if prohibit_save && SAVE_GUARDED.contains(&_i) {
                        return MouseOutcome::Consumed;
                    }
                    if pressed != self.held {
                        if pressed {
                            self.pressed_cmd = Some(_i);
                        } else if let Some(cmd) = self.pressed_cmd.take() {
                            self.begin_close();
                            return MouseOutcome::Command(cmd);
                        }
                        self.held = pressed;
                    }
                    return MouseOutcome::Consumed;
                }
                if !in_rect(x, y, ZONE) {
                    self.begin_close();
                }
                MouseOutcome::None
            }
            MenuState::Opening | MenuState::Closing => MouseOutcome::None,
        }
    }
}

