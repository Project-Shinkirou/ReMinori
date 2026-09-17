#[derive(Debug, Clone)]
pub struct AniFrame {
    pub label: String,
    pub w: u16,
    pub h: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub rgba: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct Ani {
    pub frames: Vec<AniFrame>,
}
#[derive(Debug)]
pub struct AniError(pub &'static str);
impl std::fmt::Display for AniError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ani: {}", self.0)
    }
}
impl std::error::Error for AniError {}
impl Ani {
    pub fn parse(data: &[u8]) -> Result<Self, AniError> {
        if data.len() < 8 || data[0] != 0 || data[1] != 1 || &data[4..8] != &[0, 0, 0, 0]
        {
            return Err(AniError("bad header"));
        }
        let count = u16::from_le_bytes([data[2], data[3]]) as usize;
        let mut frames = Vec::with_capacity(count);
        let mut p = 8usize;
        for _ in 0..count {
            let Some(nul) = data[p..].iter().position(|&b| b == 0) else {
                return Err(AniError("unterminated label"));
            };
            let label = crate::encoding::cp932_to_string(&data[p..p + nul]);
            p += nul + 1;
            if p + 10 > data.len() {
                return Err(AniError("truncated frame header"));
            }
            let f: [u16; 5] = core::array::from_fn(|i| {
                u16::from_le_bytes(data[p + 2 * i..p + 2 * i + 2].try_into().unwrap())
            });
            p += 10;
            let (w, h) = (f[0] as usize, f[1] as usize);
            let px_len = w
                .checked_mul(h)
                .and_then(|v| v.checked_mul(4))
                .ok_or(AniError("size overflow"))?;
            if p + px_len > data.len() {
                return Err(AniError("truncated pixels"));
            }
            let mut rgba = data[p..p + px_len].to_vec();
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            p += px_len;
            frames
                .push(AniFrame {
                    label,
                    w: f[0],
                    h: f[1],
                    flags: f[2],
                    x: f[3],
                    y: f[4],
                    rgba,
                });
        }
        if p != data.len() {
            return Err(AniError("trailing bytes"));
        }
        Ok(Ani { frames })
    }
    pub fn is_ani(data: &[u8]) -> bool {
        data.len() >= 8 && data[0] == 0 && data[1] == 1 && data[2] % 2 == 0
            && &data[4..8] == &[0, 0, 0, 0]
    }
    pub fn encode(&self) -> Result<Vec<u8>, AniError> {
        let count = u16::try_from(self.frames.len())
            .map_err(|_| AniError("too many frames"))?;
        let mut out = Vec::new();
        out.extend_from_slice(&[0x00, 0x01]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        for frame in &self.frames {
            if frame.label.contains('\0') {
                return Err(AniError("label contains NUL"));
            }
            let label = crate::encoding::string_to_cp932_strict(&frame.label)
                .ok_or(AniError("label is not representable in cp932"))?;
            let expected = usize::from(frame.w)
                .checked_mul(usize::from(frame.h))
                .and_then(|n| n.checked_mul(4))
                .ok_or(AniError("size overflow"))?;
            if frame.rgba.len() != expected {
                return Err(AniError("rgba length does not match dimensions"));
            }
            out.extend_from_slice(&label);
            out.push(0);
            for value in [frame.w, frame.h, frame.flags, frame.x, frame.y] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            for rgba in frame.rgba.chunks_exact(4) {
                out.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
            }
        }
        Ok(out)
    }
}

