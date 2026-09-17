pub mod gpu;
pub mod text;
pub const LOGICAL_W: f32 = 1024.0;
pub const LOGICAL_H: f32 = 640.0;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub phys_w: u32,
    pub phys_h: u32,
    pub scale: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}
impl Viewport {
    pub fn new(phys_w: u32, phys_h: u32) -> Self {
        let (w, h) = (phys_w.max(1) as f32, phys_h.max(1) as f32);
        let scale = (w / LOGICAL_W).min(h / LOGICAL_H).max(0.05);
        let origin_x = (w - LOGICAL_W * scale) * 0.5;
        let origin_y = (h - LOGICAL_H * scale) * 0.5;
        Viewport {
            phys_w,
            phys_h,
            scale,
            origin_x,
            origin_y,
        }
    }
    pub fn device_px(&self, logical: f32) -> f32 {
        logical * self.scale
    }
    pub fn to_phys(&self, x: f32, y: f32) -> (f32, f32) {
        (self.origin_x + x * self.scale, self.origin_y + y * self.scale)
    }
    pub fn to_logical(&self, px: f32, py: f32) -> (f32, f32) {
        ((px - self.origin_x) / self.scale, (py - self.origin_y) / self.scale)
    }
    pub fn rect_to_ndc(&self, x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        let (px0, py0) = self.to_phys(x, y);
        let (px1, py1) = self.to_phys(x + w, y + h);
        let nx0 = px0 / self.phys_w as f32 * 2.0 - 1.0;
        let nx1 = px1 / self.phys_w as f32 * 2.0 - 1.0;
        let ny0 = 1.0 - py0 / self.phys_h as f32 * 2.0;
        let ny1 = 1.0 - py1 / self.phys_h as f32 * 2.0;
        [nx0, ny0, nx1, ny1]
    }
    pub fn scissor(&self) -> (u32, u32, u32, u32) {
        let x = self.origin_x.max(0.0) as u32;
        let y = self.origin_y.max(0.0) as u32;
        let w = (LOGICAL_W * self.scale).ceil() as u32;
        let h = (LOGICAL_H * self.scale).ceil() as u32;
        (
            x,
            y,
            w.min(self.phys_w.saturating_sub(x)),
            h.min(self.phys_h.saturating_sub(y)),
        )
    }
}

