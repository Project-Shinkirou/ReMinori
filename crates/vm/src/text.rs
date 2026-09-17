pub const HANG_CHARS: &str = "．。、」』）？！";
pub fn is_hang(c: char) -> bool {
    HANG_CHARS.contains(c)
}
fn is_hang_open(c: char) -> bool {
    c == '―' || c == '｜'
}
fn is_hang_close(c: char) -> bool {
    c == '‖' || c == '／'
}
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub start_x: f32,
    pub explicit_break: bool,
}
pub struct LineBreaker {
    left: f32,
    right: f32,
    cell: f32,
    shadow: bool,
    autowrap: bool,
    pen: f32,
    hang_open: i32,
    hang_width: f32,
    lines: Vec<Line>,
}
impl LineBreaker {
    pub fn new(
        rect_left: f32,
        rect_right: f32,
        cell: f32,
        shadow: bool,
        autowrap: bool,
    ) -> Self {
        LineBreaker {
            left: rect_left,
            right: rect_right,
            cell,
            shadow,
            autowrap,
            pen: rect_left,
            hang_open: 0,
            hang_width: 0.0,
            lines: vec![
                Line { text : String::new(), start_x : rect_left, explicit_break : false,
                }
            ],
        }
    }
    fn limits(&self) -> (f32, f32) {
        let hard = self.right - self.cell + 1.0 - if self.shadow { 2.0 } else { 0.0 };
        (hard - self.cell, hard)
    }
    fn newline(&mut self, explicit: bool) {
        let start = self.left + if self.hang_open > 0 { self.hang_width } else { 0.0 };
        self.pen = start;
        self.lines
            .push(Line {
                text: String::new(),
                start_x: start,
                explicit_break: explicit,
            });
    }
    pub fn push_char(&mut self, c: char, advance: f32) {
        let extra = if self.shadow && c != '￥' && c != '№' { 2.0 } else { 0.0 };
        let adv = advance + extra;
        let (soft, hard) = self.limits();
        if !(self.pen <= soft || (self.pen < hard && (is_hang(c) || !self.autowrap))) {
            self.newline(false);
        }
        if is_hang_open(c) {
            if self.hang_open == 0 {
                self.hang_width = adv;
            }
            self.hang_open += 1;
        } else if is_hang_close(c) {
            self.hang_open -= 1;
        }
        self.lines.last_mut().unwrap().text.push(c);
        self.pen += adv;
    }
    pub fn push_break(&mut self) {
        self.newline(true);
    }
    pub fn finish(mut self) -> Vec<Line> {
        if self.lines.last().is_some_and(|l| l.text.is_empty() && !l.explicit_break) {
            self.lines.pop();
        }
        self.lines
    }
}

