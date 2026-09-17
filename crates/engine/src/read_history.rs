use std::path::PathBuf;
const MESSAGE_LIMIT: u32 = 0x1_0000;
const BYTES: usize = MESSAGE_LIMIT as usize / 8;
#[derive(Debug, Clone)]
pub struct ReadHistory {
    bits: Vec<u8>,
    path: Option<PathBuf>,
}
impl Default for ReadHistory {
    fn default() -> Self {
        Self {
            bits: vec![0; BYTES],
            path: None,
        }
    }
}
impl ReadHistory {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut history = Self::default();
        let len = bytes.len().min(BYTES);
        history.bits[..len].copy_from_slice(&bytes[..len]);
        history
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bits.clone()
    }
    pub fn load(path: PathBuf) -> Self {
        let mut history = std::fs::read(&path)
            .ok()
            .map_or_else(Self::default, |bytes| Self::from_bytes(&bytes));
        history.path = Some(path.clone());
        history
    }
    pub fn contains(&self, message_no: u32) -> bool {
        if message_no >= MESSAGE_LIMIT {
            return false;
        }
        let byte = message_no as usize / 8;
        let bit = message_no % 8;
        self.bits[byte] & (1 << bit) != 0
    }
    pub fn mark(&mut self, message_no: u32) {
        if message_no >= MESSAGE_LIMIT || self.contains(message_no) {
            return;
        }
        let byte = message_no as usize / 8;
        let bit = message_no % 8;
        self.bits[byte] |= 1 << bit;
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(err) = std::fs::write(path, &self.bits) {
            log::warn!("已读表写入失败 {}: {err}", path.display());
        }
    }
}

