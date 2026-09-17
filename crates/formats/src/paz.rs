use crate::blowfish::{negate_key, Blowfish};
use crate::paz_keys::{key_index, ARCHIVE_ORDER, DATA_KEYS_SLOT0, INDEX_KEYS_SLOT0};
use flate2::read::ZlibDecoder;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
pub const PAZ_HEADER_LEN: usize = 36;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PazEntry {
    pub name: String,
    pub offset: u32,
    pub offset_hi: u32,
    pub size: u32,
    pub stored: u32,
    pub aligned: u32,
    pub method: u32,
}
impl PazEntry {
    pub fn offset64(&self) -> u64 {
        u64::from(self.offset) | (u64::from(self.offset_hi) << 32)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PazHeader {
    pub prefix: String,
    pub index_size: u32,
    xor_key: u8,
}
#[derive(Debug, Clone)]
pub struct PazIndex {
    pub prefix: String,
    pub entries: Vec<PazEntry>,
    slot: usize,
    xor_key: u8,
    rc4_material: Option<[u8; 256]>,
}
impl PazIndex {
    pub fn parse_header(header: &[u8], xor_key: u8) -> Result<PazHeader, PazError> {
        if header.len() < PAZ_HEADER_LEN {
            return Err(PazError::BadIndex);
        }
        let mut decoded = [0u8; PAZ_HEADER_LEN];
        for (output, input) in decoded.iter_mut().zip(header.iter()) {
            *output = *input ^ xor_key;
        }
        let prefix = decoded[..32].split(|byte| *byte == 0).next().unwrap_or_default();
        let index_size = u32::from_le_bytes(decoded[32..36].try_into().unwrap());
        if index_size == 0 {
            return Err(PazError::BadIndex);
        }
        Ok(PazHeader {
            prefix: String::from_utf8_lossy(prefix).into_owned(),
            index_size,
            xor_key,
        })
    }
    pub fn parse_index(
        header: PazHeader,
        encrypted_index: &[u8],
        slot: usize,
        material: bool,
    ) -> Result<Self, PazError> {
        let index_size = header.index_size as usize;
        if index_size < 4 || encrypted_index.len() != index_size {
            return Err(PazError::BadIndex);
        }
        let mut index = encrypted_index.to_vec();
        apply_file_xor(&mut index, header.xor_key);
        Blowfish::new(&negate_key(&INDEX_KEYS_SLOT0[key_index(slot)]))
            .decrypt(&mut index);
        let count = u32::from_le_bytes(index[..4].try_into().unwrap()) as usize;
        let mut cursor = 4usize;
        let rc4_material = if material {
            if index.len() < 260 {
                return Err(PazError::BadIndex);
            }
            let mut value = [0u8; 256];
            value.copy_from_slice(&index[4..260]);
            cursor = 260;
            Some(value)
        } else {
            None
        };
        if count > index.len().saturating_sub(cursor) / 25 {
            return Err(PazError::BadIndex);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let remaining = index.get(cursor..).ok_or(PazError::BadIndex)?;
            let nul = remaining
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(PazError::BadIndex)?;
            let name = crate::encoding::cp932_to_string(&remaining[..nul]);
            cursor = cursor.checked_add(nul + 1).ok_or(PazError::BadIndex)?;
            let end = cursor.checked_add(24).ok_or(PazError::BadIndex)?;
            let metadata = index.get(cursor..end).ok_or(PazError::BadIndex)?;
            let fields: [u32; 6] = std::array::from_fn(|field| {
                let start = field * 4;
                u32::from_le_bytes(metadata[start..start + 4].try_into().unwrap())
            });
            cursor = end;
            entries
                .push(PazEntry {
                    name,
                    offset: fields[0],
                    offset_hi: fields[1],
                    size: fields[2],
                    stored: fields[3],
                    aligned: fields[4],
                    method: fields[5],
                });
        }
        Ok(Self {
            prefix: header.prefix,
            entries,
            slot,
            xor_key: header.xor_key,
            rc4_material,
        })
    }
    pub fn xor_key(&self) -> u8 {
        self.xor_key
    }
    pub fn find(&self, name: &str) -> Option<&PazEntry> {
        find_entry(&self.entries, name)
    }
    pub fn entry_read_len(
        &self,
        entry: &PazEntry,
        file_len: u64,
    ) -> Result<usize, PazError> {
        entry_read_len(self.rc4_material.is_some(), entry, file_len)
    }
    pub fn decode_entry(
        &self,
        entry: &PazEntry,
        encrypted: &mut [u8],
    ) -> Result<Vec<u8>, PazError> {
        decode_entry_impl(
            self.slot,
            self.xor_key,
            self.rc4_material.as_ref(),
            entry,
            encrypted,
        )
    }
}
#[derive(Debug)]
pub enum PazError {
    Io(std::io::Error),
    BadIndex,
    EntryNotFound(String),
    RangeOutOfBounds { offset: u64, length: usize, file_len: u64 },
    InvalidEntry { name: String, reason: String },
    UnsupportedMethod { name: String, method: u32 },
}
impl From<std::io::Error> for PazError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl std::fmt::Display for PazError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::BadIndex => write!(formatter, "bad paz index"),
            Self::EntryNotFound(name) => write!(formatter, "entry not found: {name}"),
            Self::RangeOutOfBounds { offset, length, file_len } => {
                write!(
                    formatter,
                    "paz range {offset:#x}+{length:#x} exceeds file length {file_len:#x}"
                )
            }
            Self::InvalidEntry { name, reason } => write!(formatter, "{name}: {reason}"),
            Self::UnsupportedMethod { name, method } => {
                write!(formatter, "{name}: unsupported paz method {method}")
            }
        }
    }
}
impl std::error::Error for PazError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
#[derive(Debug)]
pub struct FileRangeSource {
    file: Mutex<File>,
    len: u64,
}
impl FileRangeSource {
    pub fn open(path: &Path) -> Result<Self, PazError> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file: Mutex::new(file),
            len,
        })
    }
    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>, PazError> {
        let length64 = length as u64;
        offset
            .checked_add(length64)
            .filter(|end| *end <= self.len)
            .ok_or(PazError::RangeOutOfBounds {
                offset,
                length,
                file_len: self.len,
            })?;
        let mut bytes = vec![0; length];
        let mut file = self
            .file
            .lock()
            .map_err(|_| PazError::Io(
                std::io::Error::other("PAZ file reader lock poisoned"),
            ))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}
#[derive(Debug)]
pub struct PazArchive {
    pub prefix: String,
    pub xor_key: u8,
    pub entries: Vec<PazEntry>,
    source: FileRangeSource,
    slot: usize,
    rc4_material: Option<[u8; 256]>,
}
impl PazArchive {
    pub fn open_main(path: &Path, slot: usize, xor_key: u8) -> Result<Self, PazError> {
        Self::open_inner(path, slot, xor_key, false)
    }
    pub fn open_main_with_material(
        path: &Path,
        slot: usize,
        xor_key: u8,
    ) -> Result<Self, PazError> {
        Self::open_inner(path, slot, xor_key, true)
    }
    fn open_inner(
        path: &Path,
        slot: usize,
        xor_key: u8,
        material: bool,
    ) -> Result<Self, PazError> {
        let source = FileRangeSource::open(path)?;
        let raw_header = source.read_range(0, PAZ_HEADER_LEN)?;
        let header = PazIndex::parse_header(&raw_header, xor_key)?;
        let raw_index = source
            .read_range(PAZ_HEADER_LEN as u64, header.index_size as usize)?;
        let index = PazIndex::parse_index(header, &raw_index, slot, material)?;
        Ok(Self {
            prefix: index.prefix,
            xor_key: index.xor_key,
            entries: index.entries,
            source,
            slot: index.slot,
            rc4_material: index.rc4_material,
        })
    }
    pub fn find(&self, name: &str) -> Option<&PazEntry> {
        find_entry(&self.entries, name)
    }
    pub fn read(&self, name: &str) -> Result<Vec<u8>, PazError> {
        let entry = self
            .find(name)
            .cloned()
            .ok_or_else(|| PazError::EntryNotFound(name.to_owned()))?;
        let length = entry_read_len(
            self.rc4_material.is_some(),
            &entry,
            self.source.len(),
        )?;
        let mut encrypted = self.source.read_range(entry.offset64(), length)?;
        decode_entry_impl(
            self.slot,
            self.xor_key,
            self.rc4_material.as_ref(),
            &entry,
            &mut encrypted,
        )
    }
    pub fn group_name(slot: usize) -> &'static str {
        ARCHIVE_ORDER.get(slot).copied().unwrap_or("?")
    }
}
fn entry_name_eq(left: &str, right: &str) -> bool {
    fn canonical(byte: u8) -> u8 {
        match byte {
            b'\\' => b'/',
            other => other.to_ascii_lowercase(),
        }
    }
    let mut left = left.bytes().map(canonical);
    let mut right = right.bytes().map(canonical);
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if a == b => continue,
            _ => return false,
        }
    }
}
fn find_entry<'a>(entries: &'a [PazEntry], name: &str) -> Option<&'a PazEntry> {
    entries.iter().find(|entry| entry_name_eq(&entry.name, name))
}
fn entry_read_len(
    mov: bool,
    entry: &PazEntry,
    file_len: u64,
) -> Result<usize, PazError> {
    let offset = entry.offset64();
    if offset >= file_len {
        return Err(PazError::RangeOutOfBounds {
            offset,
            length: 0,
            file_len,
        });
    }
    let length64 = if mov {
        u64::from(entry.size)
    } else if entry.aligned >= 8 {
        u64::from(entry.aligned)
    } else {
        file_len - offset
    };
    let length = usize::try_from(length64)
        .map_err(|_| PazError::InvalidEntry {
            name: entry.name.clone(),
            reason: "entry range cannot fit in memory".into(),
        })?;
    offset
        .checked_add(length64)
        .filter(|end| *end <= file_len)
        .ok_or(PazError::RangeOutOfBounds {
            offset,
            length,
            file_len,
        })?;
    Ok(length)
}
fn apply_file_xor(bytes: &mut [u8], xor_key: u8) {
    for byte in bytes {
        *byte ^= xor_key;
    }
}
fn decode_entry_impl(
    slot: usize,
    xor_key: u8,
    rc4_material: Option<&[u8; 256]>,
    entry: &PazEntry,
    encrypted: &mut [u8],
) -> Result<Vec<u8>, PazError> {
    apply_file_xor(encrypted, xor_key);
    if let Some(material) = rc4_material {
        if encrypted.len() != entry.size as usize {
            return Err(
                invalid_length(entry, "mov", encrypted.len(), entry.size as usize),
            );
        }
        let key_text = format!(
            "{} {:08X} Uyiu4Ruy", entry.name.to_ascii_lowercase(), entry.size
        );
        let key_text = key_text.as_bytes();
        let mut key = [0u8; 256];
        for (index, byte) in key.iter_mut().enumerate() {
            *byte = material[index] ^ key_text[index % key_text.len()];
        }
        let mut key_stream = vec![0u8; 0x10000];
        crate::resource::rc4_xor(&key, &mut key_stream);
        for (index, byte) in encrypted.iter_mut().enumerate() {
            *byte ^= key_stream[index & 0xffff];
        }
        return Ok(encrypted.to_vec());
    }
    if entry.aligned >= 8 && encrypted.len() != entry.aligned as usize {
        return Err(
            invalid_length(entry, "Blowfish", encrypted.len(), entry.aligned as usize),
        );
    }
    Blowfish::new(&negate_key(&DATA_KEYS_SLOT0[key_index(slot)])).decrypt(encrypted);
    match entry.method {
        0 => take_logical_size(entry, encrypted),
        1 => {
            let stored = entry.stored as usize;
            let compressed = encrypted
                .get(..stored)
                .ok_or_else(|| PazError::InvalidEntry {
                    name: entry.name.clone(),
                    reason: format!("stored size {stored} exceeds decrypted range"),
                })?;
            let mut decoded = Vec::new();
            ZlibDecoder::new(compressed)
                .read_to_end(&mut decoded)
                .map_err(|error| PazError::InvalidEntry {
                    name: entry.name.clone(),
                    reason: format!("method=1 zlib inflate failed: {error}"),
                })?;
            take_logical_size(entry, &decoded)
        }
        method => {
            Err(PazError::UnsupportedMethod {
                name: entry.name.clone(),
                method,
            })
        }
    }
}
fn invalid_length(
    entry: &PazEntry,
    kind: &str,
    actual: usize,
    expected: usize,
) -> PazError {
    PazError::InvalidEntry {
        name: entry.name.clone(),
        reason: format!("truncated {kind} entry: received {actual}, expected {expected}"),
    }
}
fn take_logical_size(entry: &PazEntry, bytes: &[u8]) -> Result<Vec<u8>, PazError> {
    let logical = entry.size as usize;
    bytes
        .get(..logical)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| PazError::InvalidEntry {
            name: entry.name.clone(),
            reason: format!(
                "decoded size {} is shorter than logical size {logical}", bytes.len()
            ),
        })
}

