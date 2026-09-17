use crate::render::text::visible_chars;
use crate::scene::{MessageState, Scene};
use eden_vm::BacklogEntry;
pub const GAUGE_RECT: [f32; 4] = [1006.0, 496.0, 1024.0, 640.0];
pub const GAUGE_TRACK_TOP: f32 = 499.0;
pub const GAUGE_TRACK_HEIGHT: f32 = 138.0;
#[derive(Debug, Clone)]
pub struct BacklogView {
    entries: Vec<BacklogEntry>,
    index: usize,
    scene: Scene,
    pub dragging_gauge: bool,
}
impl BacklogView {
    pub fn new(entries: Vec<BacklogEntry>) -> Option<Self> {
        let index = entries.len().checked_sub(1)?;
        let scene = scene_for(&entries[index]);
        Some(Self {
            entries,
            index,
            scene,
            dragging_gauge: false,
        })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn current(&self) -> &BacklogEntry {
        &self.entries[self.index]
    }
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn older(&mut self, count: usize) {
        self.set_index(self.index.saturating_sub(count.max(1)));
    }
    pub fn newer(&mut self, count: usize) -> bool {
        let next = self.index.saturating_add(count.max(1));
        if next >= self.entries.len() {
            return false;
        }
        self.set_index(next);
        true
    }
    pub fn oldest(&mut self) {
        self.set_index(0);
    }
    pub fn newest(&mut self) {
        self.set_index(self.entries.len() - 1);
    }
    pub fn set_from_gauge_y(&mut self, y: f32) {
        let ratio = ((y - GAUGE_TRACK_TOP) / GAUGE_TRACK_HEIGHT).clamp(0.0, 1.0);
        let max = self.entries.len().saturating_sub(1);
        self.set_index((ratio * max as f32).round() as usize);
    }
    pub fn ball_y(&self) -> f32 {
        ((138 * self.index / self.entries.len()) as i32 - 7).clamp(0, 124) as f32 + 499.0
    }
    fn set_index(&mut self, index: usize) {
        self.index = index.min(self.entries.len() - 1);
        self.scene = scene_for(&self.entries[self.index]);
    }
}
fn scene_for(entry: &BacklogEntry) -> Scene {
    let mut scene = Scene::default();
    if !entry.view.bg.is_empty() {
        scene
            .stage(
                &entry.view.bg,
                entry.view.bg_x.parse().unwrap_or(0),
                entry.view.bg_y.parse().unwrap_or(0),
            );
        scene.active_tr = None;
        scene.prev_bg = None;
    }
    scene.panel_mode = entry.view.panel_mode.parse().unwrap_or(0);
    let body = entry.body.clone();
    let visible_total = visible_chars(&body);
    scene.message = Some(MessageState {
        message_no: entry.no,
        speaker: (!entry.speaker.is_empty()).then(|| entry.speaker.clone()),
        body,
        voice: (!entry.voice.is_empty()).then(|| entry.voice.clone()),
        revealed: visible_total,
        char_timer: 0,
        visible_total,
        is_read: true,
        char_interval: crate::scene::MESSAGE_SPEED_DEFAULT
            / crate::scene::MESSAGE_SPEED_DIV,
    });
    scene
}

