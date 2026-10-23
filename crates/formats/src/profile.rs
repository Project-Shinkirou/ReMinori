//! Explicit keys.v2 archive parameters. No edition-specific private keys live here.
use crate::{blowfish::Blowfish, FileRangeSource, PazEntry};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};

pub const ROLES: [&str; 8] = ["bg", "bgm", "mov", "scr", "se", "st", "sys", "voice"];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveProfile {
    schema: String,
    pub paz_version: u8,
    pub index_size_xor: u32,
    #[serde(default = "yes")]
    pub rc4_skip_crc: bool,
    pub archive_keys: BTreeMap<String, RoleKeys>,
    #[serde(default)]
    pub texture_overrides: BTreeMap<String, String>,
    #[serde(skip)]
    cache_fingerprint: u32,
}
impl std::fmt::Debug for ArchiveProfile {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.debug_struct("ArchiveProfile").field("paz_version", &self.paz_version).finish_non_exhaustive()
    }
}
fn yes() -> bool { true }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleKeys {
    index_key_hex: String,
    data_key_hex: String,
    #[serde(default)]
    type_passwords: BTreeMap<String, String>,
    #[serde(default)]
    archive_xor: Option<u32>,
    #[serde(default)]
    video_key_hex: Option<String>,
}
fn hex(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len() % 2 == 0 && text.bytes().all(|b| b.is_ascii_hexdigit()), "invalid profile key encoding");
    text.as_bytes().chunks_exact(2).map(|pair| {
        let n = |b: u8| if b.is_ascii_digit() { b - b'0' } else { b.to_ascii_lowercase() - b'a' + 10 };
        Ok(n(pair[0]) * 16 + n(pair[1]))
    }).collect()
}
fn valid_hex(text: &str, min: usize, max: usize) -> bool {
    text.len() % 2 == 0 && (min..=max).contains(&(text.len() / 2))
        && text.bytes().all(|b| b.is_ascii_hexdigit())
}
impl ArchiveProfile {
    pub fn read(path: &Path) -> Result<Arc<Self>> {
        let file = std::fs::File::open(path).context("cannot open archive profile")?;
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "archive profile exceeds 64 KiB");
        // Never expose parser diagnostics: TOML errors may quote a private key.
        let text = std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("archive profile is not UTF-8"))?;
        let mut profile: Self = toml::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| anyhow::anyhow!("invalid keys.v2 archive profile"))?;
        ensure!(profile.schema == "shinki.emu.musica.keys.v2", "unsupported archive profile schema");
        ensure!(profile.paz_version <= 2, "unsupported PAZ version");
        ensure!(profile.archive_keys.len() == ROLES.len(), "profile must define all eight archive roles");
        for role in ROLES {
            let keys = profile.archive_keys.get(role).context("missing archive role")?;
            ensure!(valid_hex(&keys.index_key_hex, 4, 56), "invalid index key encoding or length");
            ensure!(valid_hex(&keys.data_key_hex, if role == "mov" { 0 } else { 4 }, 56), "invalid data key encoding or length");
            if let Some(key) = &keys.video_key_hex { ensure!(valid_hex(key, 256, 256), "invalid video key encoding or length"); }
            ensure!(keys.archive_xor.is_none_or(|xor| xor == profile.index_size_xor), "archive XOR conflicts with profile index XOR");
            ensure!(keys.type_passwords.iter().all(|(kind, password)| {
                ["png", "ogg", "sc", "avi"].contains(&kind.as_str()) && password.len() <= 256 && !password.contains('\0')
            }), "invalid type password parameters");
        }
        for (source, replacement) in &profile.texture_overrides {
            ensure!(!source.is_empty() && safe_relative(replacement), "unsafe texture override path");
        }
        profile.cache_fingerprint = crc32fast::hash(&bytes);
        Ok(Arc::new(profile))
    }
    pub fn cache_tag(&self) -> u32 { self.cache_fingerprint }
}
pub fn safe_relative(path: &str) -> bool {
    !path.is_empty() && !path.contains(':') && !path.contains('\0')
        && !path.starts_with(['/', '\\'])
        && path.replace('\\', "/").split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

pub struct ProfileArchive {
    source: FileRangeSource,
    pub entries: Vec<PazEntry>,
    profile: Arc<ArchiveProfile>,
    role: String,
    xor: u8,
    video_key: Option<Vec<u8>>,
    data_cipher: Option<Blowfish>,
}
impl ProfileArchive {
    pub fn open(path: &Path, role: &str, profile: Arc<ArchiveProfile>) -> Result<Self> {
        let keys = profile.archive_keys.get(role).context("unknown archive role")?;
        let source = FileRangeSource::open(path)?;
        let (offset, size, xor) = if profile.paz_version == 0 {
            let header = source.read_range(0, 4)?;
            (4, u32::from_le_bytes(header.try_into().unwrap()), 0)
        } else {
            let header = source.read_range(32, 4)?;
            let raw = u32::from_le_bytes(header.try_into().unwrap());
            let xor = (raw >> 24) as u8;
            let mask = u32::from_le_bytes([xor; 4]);
            ensure!(profile.index_size_xor == 0 || profile.index_size_xor == mask, "profile/header index XOR mismatch");
            (36, raw ^ mask, xor)
        };
        ensure!(size >= 8 && size <= 64 * 1024 * 1024 && size % 8 == 0, "invalid PAZ index size");
        let mut index = source.read_range(offset, size as usize)?;
        for b in &mut index { *b ^= xor; }
        Blowfish::new(&hex(&keys.index_key_hex)?).decrypt(&mut index);
        let mut cursor = 0;
        fn word(data: &[u8], cursor: &mut usize) -> Result<u32> {
            let bytes = data.get(*cursor..*cursor + 4).context("truncated PAZ index")?;
            *cursor += 4;
            Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
        }
        let count = word(&index, &mut cursor)? as usize;
        let mut video_key = if role == "mov" {
            let key = index.get(cursor..cursor + 256).context("truncated movie key")?.to_vec();
            cursor += 256;
            Some(key)
        } else { None };
        if role == "mov" {
            if let Some(key) = &keys.video_key_hex { video_key = Some(hex(key)?); }
        }
        ensure!(count <= index.len().saturating_sub(cursor) / 25, "invalid PAZ entry count");
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let remaining = index.get(cursor..).context("truncated PAZ index")?;
            let end = remaining.iter().position(|b| *b == 0).context("unterminated PAZ name")?;
            ensure!(end > 0 && end <= 4096, "invalid PAZ name size");
            let name = crate::encoding::cp932_to_string(&remaining[..end]);
            ensure!(safe_relative(&name), "unsafe PAZ entry path");
            cursor += end + 1;
            let entry = PazEntry {
                name, offset: word(&index, &mut cursor)?, offset_hi: word(&index, &mut cursor)?,
                size: word(&index, &mut cursor)?, stored: word(&index, &mut cursor)?,
                aligned: word(&index, &mut cursor)?, method: word(&index, &mut cursor)?,
            };
            ensure!(entry.size <= 512 * 1024 * 1024 && entry.stored <= entry.aligned
                && (role == "mov" || entry.aligned % 8 == 0)
                && entry.offset64().checked_add(entry.aligned as u64).is_some_and(|end| end <= source.len()),
                "PAZ entry exceeds archive bounds");
            entries.push(entry);
        }
        let data_cipher = if role == "mov" { None } else { Some(Blowfish::new(&hex(&keys.data_key_hex)?)) };
        Ok(Self { source, entries, profile, role: role.into(), xor, video_key, data_cipher })
    }
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let entry = self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).context("PAZ entry not found")?;
        let mut bytes = self.source.read_range(entry.offset64(), entry.aligned as usize)?;
        let keys = &self.profile.archive_keys[&self.role];
        for b in &mut bytes { *b ^= self.xor; }
        let lower = entry.name.to_lowercase();
        let kind = if self.role == "mov" { "avi" }
            else if lower.ends_with(".png") { "png" }
            else if lower.ends_with(".ogg") || self.role == "voice" || self.role == "se" { "ogg" }
            else if lower.ends_with(".sc") { "sc" } else { "" };
        let password = if entry.method == 0 { keys.type_passwords.get(kind) } else { None };
        let material = if self.profile.paz_version > 0 && (self.video_key.is_some() || password.is_some()) {
            format!("{} {:08X} {}", lower, entry.size, password.map_or("", String::as_str))
        } else { String::new() };
        let (material, _, malformed) = encoding_rs::SHIFT_JIS.encode(&material);
        ensure!(!malformed, "PAZ RC4 material is not CP932");
        if let Some(video_key) = &self.video_key {
            if self.profile.paz_version == 0 {
                let mut inverse = [0; 256];
                for (index, value) in video_key.iter().enumerate() { inverse[*value as usize] = index as u8; }
                for b in &mut bytes { *b = inverse[*b as usize]; }
            } else {
                let key: Vec<u8> = (0..256).map(|i| video_key[i] ^ material[i % material.len()]).collect();
                let mut stream = vec![0; bytes.len().min(65536)];
                crate::resource::rc4_xor(&key, &mut stream);
                if !stream.is_empty() { for (i, b) in bytes.iter_mut().enumerate() { *b ^= stream[i % stream.len()]; } }
            }
        } else {
            self.data_cipher.as_ref().context("missing data cipher")?.decrypt(&mut bytes);
            if self.profile.paz_version > 0 && password.is_some() {
                let skip = if self.profile.paz_version >= 2 && self.profile.rc4_skip_crc {
                    ((crc32fast::hash(&material) >> 12) & 255) as usize
                } else { 0 };
                rc4_skip(&material, skip, &mut bytes);
            }
        }
        if entry.method != 0 {
            let compressed = bytes.get(..entry.stored as usize).context("invalid stored length")?;
            let mut decoded = Vec::with_capacity(entry.size as usize);
            flate2::read::ZlibDecoder::new(compressed).take(entry.size as u64 + 1).read_to_end(&mut decoded)?;
            bytes = decoded;
            ensure!(bytes.len() == entry.size as usize, "inflated PAZ entry has wrong length");
        }
        ensure!(bytes.len() >= entry.size as usize, "truncated decoded PAZ entry");
        bytes.truncate(entry.size as usize);
        Ok(bytes)
    }
}
fn rc4_skip(key: &[u8], skip: usize, bytes: &mut [u8]) {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0;
    for i in 0..256 { j = (j + s[i] as usize + key[i % key.len()] as usize) & 255; s.swap(i, j); }
    let (mut i, mut j) = (0, 0);
    for n in 0..skip + bytes.len() {
        i = (i + 1) & 255; j = (j + s[i] as usize) & 255; s.swap(i, j);
        if n >= skip { bytes[n - skip] ^= s[(s[i] as usize + s[j] as usize) & 255]; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_resource_paths_are_confined() {
        for bad in ["../asset.png", "C:/asset.png", "/asset.png", "a/../b", "a\\..\\b", ""] { assert!(!safe_relative(bad)); }
        assert!(safe_relative("images/title.png"));
    }
    #[test]
    fn rc4_skip_matches_full_keystream() {
        let key = b"public fixture";
        let mut full = vec![0; 300];
        crate::resource::rc4_xor(key, &mut full);
        let mut tail = vec![0; 45];
        rc4_skip(key, 255, &mut tail);
        assert_eq!(tail, full[255..]);
    }
    #[test]
    fn profile_archive_decodes_public_blowfish_rc4_and_packed_entries() {
        use std::io::Write;
        for version in [1u8, 2] {
            for packed in [false, true] {
                let payload = b".message public fixture\n.end\n";
                let name = "test.sc";
                let password = "public-password";
                let mut data = if packed {
                    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(payload).unwrap();
                    encoder.finish().unwrap()
                } else { payload.to_vec() };
                let stored = data.len();
                data.resize(data.len().div_ceil(8) * 8, 0);
                if !packed {
                    let material = format!("{name} {:08X} {password}", payload.len());
                    let skip = if version == 2 { ((crc32fast::hash(material.as_bytes()) >> 12) & 255) as usize } else { 0 };
                    rc4_skip(material.as_bytes(), skip, &mut data);
                }
                Blowfish::new(&[0x22; 32]).encrypt(&mut data);
                let index_len = (4usize + name.len() + 1 + 24).div_ceil(8) * 8;
                let offset = 36 + index_len;
                let mut index = 1u32.to_le_bytes().to_vec();
                index.extend_from_slice(name.as_bytes());
                index.push(0);
                for field in [offset as u32, 0, payload.len() as u32, stored as u32, data.len() as u32, packed as u32] {
                    index.extend_from_slice(&field.to_le_bytes());
                }
                index.resize(index_len, 0);
                Blowfish::new(&[0x11; 32]).encrypt(&mut index);
                for byte in index.iter_mut().chain(data.iter_mut()) { *byte ^= 0xaa; }
                let mut archive = vec![0; 32];
                archive.extend_from_slice(&((index_len as u32) ^ 0xaaaaaaaa).to_le_bytes());
                archive.extend(index);
                archive.extend(data);
                let profile = Arc::new(ArchiveProfile {
                    schema: "shinki.emu.musica.keys.v2".into(), paz_version: version,
                    index_size_xor: 0xaaaaaaaa, rc4_skip_crc: true,
                    archive_keys: BTreeMap::from([("scr".into(), RoleKeys {
                        index_key_hex: "11".repeat(32), data_key_hex: "22".repeat(32),
                        type_passwords: BTreeMap::from([("sc".into(), password.into())]),
                        archive_xor: None, video_key_hex: None,
                    })]), texture_overrides: BTreeMap::new(), cache_fingerprint: 0,
                });
                let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
                let path = std::env::temp_dir().join(format!("reminori-paz-{}-{nonce}.paz", std::process::id()));
                std::fs::write(&path, archive).unwrap();
                let archive = ProfileArchive::open(&path, "scr", profile).unwrap();
                assert_eq!(archive.read(name).unwrap(), payload);
                drop(archive);
                std::fs::remove_file(path).unwrap();
            }
        }
    }
}
