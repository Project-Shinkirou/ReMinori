use fontdue::Font;
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub px_glyph_w: u32,
    pub px_glyph_h: u32,
    pub advance: f32,
    pub xoff: f32,
    pub yoff: f32,
}
const ATLAS_W: u32 = 2048;
const ATLAS_H: u32 = 1024;
pub const DEFAULT_FACE: &str = "ＭＳ Ｐゴシック";
fn has_glyph(font: &Font, c: char) -> bool {
    let m = font.metrics(c, 33.0);
    let (_, bmp) = font.rasterize(c, 33.0);
    m.advance_width > 20.0 && bmp.iter().filter(|&&b| b > 16).count() > 100
}
pub struct GlyphAtlas {
    font: Font,
    faces: Vec<String>,
    face: String,
    data: Vec<u8>,
    cursor: (u32, u32, u32),
    glyphs: HashMap<(char, u16), GlyphSlot>,
    pub dirty: bool,
}
impl GlyphAtlas {
    pub fn from_system() -> anyhow::Result<Self> {
        Self::from_system_named("")
    }
    pub fn from_system_named(preferred: &str) -> anyhow::Result<Self> {
        #[cfg(not(windows))]
        let _ = preferred;
        #[cfg(windows)]
        {
            let mut faces = crate::platform::windows::system_font_faces();
            let mut preferred_faces = Vec::new();
            if !preferred.is_empty() {
                preferred_faces.push(preferred.to_string());
            }
            preferred_faces.push(DEFAULT_FACE.to_string());
            preferred_faces.extend(faces.iter().cloned());
            for face in preferred_faces {
                let Ok(font) = load_windows_face(&face) else {
                    continue;
                };
                if !faces.iter().any(|candidate| candidate == &face) {
                    faces.push(face.clone());
                }
                log::info!("字体: {face}");
                return Ok(Self::new(font, faces, face));
            }
        }
        let candidates: &[(&str, u32)] = &[
            (r"C:\Windows\Fonts\msgothic.ttc", 2),
            (r"C:\Windows\Fonts\msgothic.ttc", 0),
            (r"C:\Windows\Fonts\YuGothM.ttc", 0),
            (r"C:\Windows\Fonts\meiryo.ttc", 0),
            (r"C:\Windows\Fonts\BIZ-UDGothicR.ttc", 0),
            (r"C:\Windows\Fonts\msyh.ttc", 0),
            (r"C:\Windows\Fonts\simhei.ttf", 0),
            (r"C:\Windows\Fonts\GOTHIC.TTF", 0),
        ];
        let mut last = String::new();
        for &(path, idx) in candidates {
            let Ok(bytes) = std::fs::read(path) else {
                last = format!("{path}: 读取失败");
                continue;
            };
            let settings = fontdue::FontSettings {
                collection_index: idx,
                ..Default::default()
            };
            match Font::from_bytes(bytes, settings) {
                Ok(font) if has_glyph(&font, '手') && has_glyph(&font, 'あ') => {
                    log::info!("字体: {path}#{idx}");
                    return Ok(Self::new(font, Vec::new(), DEFAULT_FACE.into()));
                }
                Ok(_) => last = format!("{path}#{idx}: 缺日文字形"),
                Err(e) => last = format!("{path}#{idx}: {e}"),
            }
        }
        anyhow::bail!("无可用日文字体（{last}）")
    }
    pub fn from_bytes(bytes: Vec<u8>) -> anyhow::Result<Self> {
        Self::from_bytes_named(bytes, "")
    }
    pub fn from_bytes_named(
        bytes: Vec<u8>,
        name: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let font = Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let name = name.into();
        log::info!("字体: {name}");
        #[cfg(windows)]
        let faces = crate::platform::windows::system_font_faces();
        #[cfg(not(windows))]
        let faces = Vec::new();
        Ok(Self::new(font, faces, name))
    }
    fn new(font: Font, faces: Vec<String>, face: String) -> Self {
        GlyphAtlas {
            font,
            faces,
            face,
            data: vec![0u8; (ATLAS_W * ATLAS_H) as usize],
            cursor: (1, 1, 0),
            glyphs: HashMap::new(),
            dirty: true,
        }
    }
    pub fn face_name(&self) -> &str {
        &self.face
    }
    pub fn cycle_face(&mut self, direction: i32) -> anyhow::Result<String> {
        if self.faces.is_empty() {
            anyhow::bail!("系统字体枚举为空");
        }
        let current = self.faces.iter().position(|name| name == &self.face).unwrap_or(0);
        for step in 1..=self.faces.len() {
            let index = (current as i32 + direction.signum() * step as i32)
                .rem_euclid(self.faces.len() as i32) as usize;
            let candidate = self.faces[index].clone();
            if self.set_face(&candidate).is_ok() {
                return Ok(candidate);
            }
        }
        anyhow::bail!("没有可切换的 TrueType 日文字体")
    }
    pub fn set_face(&mut self, face: &str) -> anyhow::Result<()> {
        if face == self.face {
            return Ok(());
        }
        #[cfg(windows)]
        let font = load_windows_face(face)?;
        #[cfg(not(windows))]
        anyhow::bail!("运行平台不支持系统字体切换: {face}");
        #[cfg(windows)]
        {
            self.font = font;
            self.face = face.to_string();
            if !self.faces.iter().any(|candidate| candidate == face) {
                self.faces.push(face.to_string());
            }
            self.data.fill(0);
            self.cursor = (1, 1, 0);
            self.glyphs.clear();
            self.dirty = true;
            Ok(())
        }
    }
    pub fn atlas_dims(&self) -> (u32, u32) {
        (ATLAS_W, ATLAS_H)
    }
    pub fn atlas_data(&self) -> &[u8] {
        &self.data
    }
    pub fn glyph(&mut self, c: char, logical_size: f32, scale: f32) -> GlyphSlot {
        let px = ((logical_size * scale).round().clamp(4.0, 256.0)) as u16;
        if let Some(slot) = self.glyphs.get(&(c, px)) {
            return *slot;
        }
        let size = px as f32;
        let metrics = self.font.metrics(c, size);
        let (gw, gh) = (metrics.width as u32, metrics.height as u32);
        let (sw, sh) = (gw + 2, gh + 2);
        let placed = self.place(sw, sh);
        match placed {
            Some((x, y)) => {
                let (_, bitmap) = self.font.rasterize(c, size);
                for row in 0..gh {
                    for col in 0..gw {
                        let src = (row * gw + col) as usize;
                        let dst = ((y + 1 + row) * ATLAS_W + x + 1 + col) as usize;
                        if src < bitmap.len() {
                            self.data[dst] = bitmap[src];
                        }
                    }
                }
                let inv = if scale > 0.0 { 1.0 / scale } else { 1.0 };
                let slot = GlyphSlot {
                    x,
                    y,
                    w: sw,
                    h: sh,
                    px_glyph_w: gw,
                    px_glyph_h: gh,
                    advance: metrics.advance_width * inv,
                    xoff: metrics.xmin as f32 * inv,
                    yoff: metrics.ymin as f32 * inv,
                };
                self.glyphs.insert((c, px), slot);
                self.dirty = true;
                slot
            }
            None => {
                log::warn!("字形图集满，重建（{} 字形）", self.glyphs.len());
                self.data.fill(0);
                self.cursor = (1, 1, 0);
                self.glyphs.clear();
                self.dirty = true;
                self.glyph(c, logical_size, scale)
            }
        }
    }
    fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w >= ATLAS_W || h >= ATLAS_H {
            return None;
        }
        let (mut x, mut y, mut row_h) = self.cursor;
        if x + w >= ATLAS_W {
            y += row_h + 1;
            x = 1;
            row_h = 0;
        }
        if y + h >= ATLAS_H {
            return None;
        }
        self.cursor = (x + w + 1, y, row_h.max(h));
        Some((x, y))
    }
    pub fn advance_logical(&mut self, c: char, logical_size: f32, scale: f32) -> f32 {
        let slot = self.glyph(c, logical_size, scale);
        if slot.advance > 0.0 {
            slot.advance
        } else if c.is_ascii() {
            logical_size * 0.5
        } else {
            logical_size
        }
    }
    pub fn ascent_logical(&self, logical_size: f32, scale: f32) -> f32 {
        self.font
            .horizontal_line_metrics(logical_size * scale)
            .map(|m| m.ascent / scale.max(1e-6))
            .unwrap_or(logical_size * 0.88)
    }
}
#[cfg(windows)]
fn load_windows_face(face: &str) -> anyhow::Result<Font> {
    let bytes = crate::platform::windows::system_font_data(face)?;
    let collection_index = collection_index_for_face(&bytes, face)?;
    let settings = fontdue::FontSettings {
        collection_index,
        ..Default::default()
    };
    let font = Font::from_bytes(bytes, settings)
        .map_err(|e| anyhow::anyhow!("{face}: {e}"))?;
    if !has_glyph(&font, '手') || !has_glyph(&font, 'あ') {
        anyhow::bail!("{face}: 缺日文字形");
    }
    Ok(font)
}
#[cfg(windows)]
fn collection_index_for_face(bytes: &[u8], requested_face: &str) -> anyhow::Result<u32> {
    let Some(face_count) = ttf_parser::fonts_in_collection(bytes) else {
        ttf_parser::Face::parse(bytes, 0)
            .map_err(|e| {
                anyhow::anyhow!(
                    "{requested_face}: invalid font data ({} bytes, tag {:02X?}): {e:?}",
                    bytes.len(), bytes.get(..4).unwrap_or(bytes)
                )
            })?;
        return Ok(0);
    };
    const FACE_NAME_IDS: [u16; 5] = [
        ttf_parser::name_id::FAMILY,
        ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
        ttf_parser::name_id::FULL_NAME,
        ttf_parser::name_id::COMPATIBLE_FULL,
        ttf_parser::name_id::WWS_FAMILY,
    ];
    for index in 0..face_count {
        let Ok(face) = ttf_parser::Face::parse(bytes, index) else {
            continue;
        };
        let matches = face
            .names()
            .into_iter()
            .any(|name| {
                FACE_NAME_IDS.contains(&name.name_id)
                    && name
                        .to_string()
                        .is_some_and(|name| name.eq_ignore_ascii_case(requested_face))
            });
        if matches {
            return Ok(index);
        }
    }
    anyhow::bail!(
        "{requested_face}: face not found in {face_count}-face font collection"
    )
}

