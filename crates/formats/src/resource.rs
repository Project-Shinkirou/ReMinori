#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Script,
    Image,
    Audio,
}
impl ResourceKind {
    pub fn rc4_string(self) -> &'static str {
        match self {
            ResourceKind::Script => "N426Fd94",
            ResourceKind::Image => "p37j344s",
            ResourceKind::Audio => "ol0lOrAf",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".sc") {
            Some(ResourceKind::Script)
        } else if lower.ends_with(".png") {
            Some(ResourceKind::Image)
        } else if lower.ends_with(".ogg") || !lower.contains('.') {
            Some(ResourceKind::Audio)
        } else {
            None
        }
    }
}
pub fn key_string(name: &str, size: u32, kind: ResourceKind) -> String {
    format!("{} {:08X} {}", name.to_ascii_lowercase(), size, kind.rc4_string())
}
pub fn rc4_xor(key: &[u8], data: &mut [u8]) {
    let mut s: [u8; 256] = core::array::from_fn(|i| i as u8);
    let mut j: usize = 0;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 0xFF;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    for b in data.iter_mut() {
        i = (i + 1) & 0xFF;
        j = (j + s[i] as usize) & 0xFF;
        s.swap(i, j);
        *b ^= s[(s[i] as usize + s[j] as usize) & 0xFF];
    }
}
pub fn decode(name: &str, size: u32, kind: ResourceKind, data: &mut [u8]) {
    let ks = key_string(name, size, kind);
    rc4_xor(ks.as_bytes(), data);
}

