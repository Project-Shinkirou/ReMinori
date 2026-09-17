use crate::font::GlyphAtlas;
use crate::render::Viewport;
use eden_vm::text::LineBreaker;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub atlas: [u32; 4],
    pub rect: [f32; 4],
    pub color: [f32; 4],
}
pub const SHADOW_ALPHA: f32 = 0xBF as f32 / 255.0;
pub const SHADOW_OFFSET: f32 = 2.0;
pub const TEXT_SHADOW: bool = true;
#[derive(Debug, Clone, Copy)]
pub struct TextBox {
    pub ox: f32,
    pub oy: f32,
    pub rect: [f32; 4],
}
pub const MSG_BOX_PANEL1: TextBox = TextBox {
    ox: 0.0,
    oy: 457.0,
    rect: [104.0, 39.0, 918.0, 183.0],
};
pub const NAME_BOX_PANEL1: TextBox = TextBox {
    ox: 0.0,
    oy: 457.0,
    rect: [104.0, 39.0, 918.0, 167.0],
};
pub const MSG_BOX_BAR: TextBox = TextBox {
    ox: 0.0,
    oy: 565.0,
    rect: [116.0, 5.0, 996.0, 85.0],
};
pub const NAME_BOX_BAR: TextBox = TextBox {
    ox: 0.0,
    oy: 565.0,
    rect: [28.0, 5.0, 116.0, 85.0],
};
pub const MSG_BOX_FULL: TextBox = TextBox {
    ox: 0.0,
    oy: 0.0,
    rect: [182.0, 32.0, 930.0, 608.0],
};
pub const NAME_BOX_FULL: TextBox = TextBox {
    ox: 0.0,
    oy: 0.0,
    rect: [94.0, 32.0, 182.0, 608.0],
};
pub const MSG_BOX_FULL2: TextBox = TextBox {
    ox: 0.0,
    oy: 0.0,
    rect: [138.0, 32.0, 886.0, 608.0],
};
#[derive(Debug, Clone, PartialEq)]
struct BaseTok {
    c: char,
    ruby: Option<String>,
}
fn body_tokens(body: &str) -> Vec<Vec<BaseTok>> {
    let mut out: Vec<Vec<BaseTok>> = vec![Vec::new()];
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' || c == '￥' {
            if let Some(&ctrl) = chars.peek() {
                match ctrl {
                    'n' | 'N' => {
                        chars.next();
                        out.push(Vec::new());
                        continue;
                    }
                    'v' | 'a' | 'p' | 'w' => {
                        chars.next();
                        continue;
                    }
                    _ => {}
                }
            }
        }
        if c == '{' {
            let mut ruby = String::new();
            for rc in chars.by_ref() {
                if rc == '}' {
                    break;
                }
                ruby.push(rc);
            }
            if let Some(last) = out.last_mut().unwrap().last_mut() {
                if last.ruby.is_none() {
                    last.ruby = Some(ruby);
                }
            }
            continue;
        }
        out.last_mut().unwrap().push(BaseTok { c, ruby: None });
    }
    out
}
pub fn layout(
    atlas: &mut GlyphAtlas,
    vp: &Viewport,
    text: &str,
    box_: &TextBox,
    size: f32,
    ruby_size: f32,
    color: [f32; 4],
) -> TextOut {
    layout_ex(atlas, vp, text, box_, size, ruby_size, color, TEXT_SHADOW, 0, None)
}
#[allow(clippy::too_many_arguments)]
pub fn layout_ex(
    atlas: &mut GlyphAtlas,
    vp: &Viewport,
    text: &str,
    box_: &TextBox,
    size: f32,
    ruby_size: f32,
    color: [f32; 4],
    shadow: bool,
    first_line: usize,
    reveal: Option<usize>,
) -> TextOut {
    let segments = body_tokens(text);
    let [left, top, right, _bottom] = box_.rect;
    let mut width_map = std::collections::HashMap::new();
    for t in segments.iter().flatten() {
        width_map
            .entry(t.c)
            .or_insert_with(|| atlas.advance_logical(t.c, size, vp.scale));
    }
    let mut breaker = LineBreaker::new(left, right, size, shadow, true);
    for (si, seg) in segments.iter().enumerate() {
        if si > 0 {
            breaker.push_break();
        }
        for t in seg {
            let w = width_map.get(&t.c).copied().unwrap_or(0.0);
            breaker.push_char(t.c, w);
        }
    }
    let lines = breaker.finish();
    let scale = vp.scale.max(1e-6);
    let ascent = atlas.ascent_logical(size, scale).round();
    let ruby_ascent = atlas.ascent_logical(ruby_size, scale).round();
    let line_height = size + ruby_size - 1.0;
    let ruby_span = ((size + 3.0) / 4.0).floor() * 4.0;
    let flat: Vec<&BaseTok> = segments.iter().flatten().collect();
    let mut tok_iter = flat.into_iter();
    let mut quads = Vec::new();
    let mut drawn = 0usize;
    let mut cursor: Option<(f32, f32)> = None;
    'outer: for (li, line) in lines.iter().enumerate() {
        let line_top = box_.oy + top + (li + first_line) as f32 * line_height;
        let baseline = line_top + ruby_size + ascent;
        let mut pen = box_.ox + line.start_x;
        for c in line.text.chars() {
            let mut ruby = None;
            if let Some(t) = tok_iter.next() {
                debug_assert_eq!(t.c, c, "折行序列与基元序列错位");
                ruby = t.ruby.clone();
            }
            let slot = atlas.glyph(c, size, scale);
            let adv = if slot.advance > 0.0 {
                slot.advance
            } else if c.is_ascii() {
                size * 0.5
            } else {
                size
            };
            let extra = if shadow && c != '￥' && c != '№' {
                SHADOW_OFFSET
            } else {
                0.0
            };
            if let Some(n) = reveal {
                if drawn >= n {
                    continue;
                }
            }
            if slot.px_glyph_w > 0 && slot.px_glyph_h > 0 {
                let gx = pen + slot.xoff;
                let gw = slot.px_glyph_w as f32 / scale;
                let gh = slot.px_glyph_h as f32 / scale;
                let gy = baseline - (slot.yoff + gh);
                if shadow {
                    quads
                        .push(GlyphQuad {
                            atlas: [
                                slot.x + 1,
                                slot.y + 1,
                                slot.px_glyph_w,
                                slot.px_glyph_h,
                            ],
                            rect: [gx + SHADOW_OFFSET, gy + SHADOW_OFFSET, gw, gh],
                            color: [0.0, 0.0, 0.0, SHADOW_ALPHA],
                        });
                }
                quads
                    .push(GlyphQuad {
                        atlas: [
                            slot.x + 1,
                            slot.y + 1,
                            slot.px_glyph_w,
                            slot.px_glyph_h,
                        ],
                        rect: [gx, gy, gw, gh],
                        color,
                    });
            }
            if let Some(r) = ruby {
                let est: f32 = r
                    .chars()
                    .map(|rc| if rc.is_ascii() { 0.5 } else { 1.0 })
                    .sum::<f32>() * ruby_size;
                let local_x = pen - box_.ox;
                let mut off = (ruby_span - est) * 0.5;
                if local_x + off < 0.0 {
                    off = 0.0;
                }
                let limit = right - est - local_x;
                if off > limit {
                    off = limit;
                }
                let rbaseline = line_top + ruby_ascent;
                let mut rpen = local_x + off + box_.ox;
                for rc in r.chars() {
                    let rs = atlas.glyph(rc, ruby_size, scale);
                    if rs.px_glyph_w > 0 && rs.px_glyph_h > 0 {
                        let rw = rs.px_glyph_w as f32 / scale;
                        let rh = rs.px_glyph_h as f32 / scale;
                        let (rx, ry) = (rpen + rs.xoff, rbaseline - (rs.yoff + rh));
                        if shadow {
                            quads
                                .push(GlyphQuad {
                                    atlas: [rs.x + 1, rs.y + 1, rs.px_glyph_w, rs.px_glyph_h],
                                    rect: [rx + SHADOW_OFFSET, ry + SHADOW_OFFSET, rw, rh],
                                    color: [0.0, 0.0, 0.0, SHADOW_ALPHA],
                                });
                        }
                        quads
                            .push(GlyphQuad {
                                atlas: [rs.x + 1, rs.y + 1, rs.px_glyph_w, rs.px_glyph_h],
                                rect: [rx, ry, rw, rh],
                                color,
                            });
                    }
                    rpen += if rc.is_ascii() { ruby_size * 0.5 } else { ruby_size };
                }
            }
            cursor = Some((pen + adv + extra, line_top));
            drawn += 1;
            if reveal.is_some_and(|n| drawn >= n) {
                break 'outer;
            }
            pen += adv + extra;
        }
    }
    TextOut { quads, cursor }
}
#[derive(Debug, Clone, Default)]
pub struct TextOut {
    pub quads: Vec<GlyphQuad>,
    pub cursor: Option<(f32, f32)>,
}
pub fn visible_chars(text: &str) -> usize {
    body_tokens(text).iter().map(|s| s.len()).sum()
}
pub fn layout_fixed(
    atlas: &mut GlyphAtlas,
    vp: &Viewport,
    text: &str,
    _box_: &TextBox,
    size: f32,
    color: [f32; 4],
) -> TextOut {
    layout(atlas, vp, text, &MSG_BOX_PANEL1, size, 10.0, color)
}
pub fn speaker_display_for_panel(speaker: &str, panel_mode: i64) -> Option<String> {
    if panel_mode == 4 {
        return None;
    }
    if let Some(rest) = speaker.strip_prefix('@') {
        let _ = rest;
        if panel_mode == 1 {
            Some("【？？？】".to_string())
        } else {
            Some("？？？".to_string())
        }
    } else if speaker.starts_with('#') || speaker.is_empty() {
        None
    } else if panel_mode == 1 {
        Some(format!("【{speaker}】"))
    } else {
        Some(speaker.to_string())
    }
}
pub fn speaker_display(speaker: &str) -> Option<String> {
    speaker_display_for_panel(speaker, 1)
}

