use crate::blowfish::Blowfish;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::collections::BTreeMap;
use std::io::{Read, Write};
const KEY: &[u8; 6] = b"minori";
fn cipher() -> Blowfish {
    let key: Vec<u8> = KEY.iter().map(|byte| byte.wrapping_neg()).collect();
    Blowfish::new(&key)
}
pub fn decode(bytes: &[u8]) -> anyhow::Result<BTreeMap<String, String>> {
    if bytes.len() < 8 || bytes.len() % 8 != 0 {
        anyhow::bail!("system.dat length is not an encrypted block stream");
    }
    let mut encrypted = bytes.to_vec();
    cipher().decrypt(&mut encrypted);
    let plain_len = u32::from_le_bytes(encrypted[..4].try_into().unwrap()) as usize;
    if plain_len == 0 || plain_len > 64 * 1024 * 1024 {
        anyhow::bail!("invalid system.dat uncompressed length: {plain_len}");
    }
    let mut decoder = ZlibDecoder::new(&encrypted[4..]);
    let mut plain = Vec::with_capacity(plain_len);
    decoder.read_to_end(&mut plain)?;
    if plain.len() != plain_len {
        anyhow::bail!(
            "system.dat length mismatch: header={plain_len}, decoded={}", plain.len()
        );
    }
    if plain.last().copied() != Some(0) {
        anyhow::bail!("system.dat TSV is not NUL terminated");
    }
    let text = std::str::from_utf8(&plain[..plain.len() - 1])?;
    let mut vars = BTreeMap::new();
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('\t')
            .ok_or_else(|| anyhow::anyhow!("malformed system.dat TSV line: {line:?}"))?;
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}
pub fn encode(vars: &BTreeMap<String, String>) -> anyhow::Result<Vec<u8>> {
    let mut tsv = Vec::new();
    for (key, value) in vars {
        if key.contains(['\0', '\n', '\r', '\t']) || value.contains(['\0', '\n', '\r']) {
            anyhow::bail!("system.dat key/value contains a record delimiter");
        }
        tsv.extend_from_slice(key.as_bytes());
        tsv.push(b'\t');
        tsv.extend_from_slice(value.as_bytes());
        tsv.push(b'\n');
    }
    tsv.push(0);
    let mut compressed = Vec::new();
    ZlibEncoder::new(&mut compressed, Compression::default()).write_all(&tsv)?;
    let total = 4usize
        .checked_add(compressed.len())
        .ok_or_else(|| anyhow::anyhow!("system.dat compressed length overflow"))?;
    let padded = (total + 7) & !7;
    let mut out = vec![0u8; padded];
    out[..4].copy_from_slice(&(tsv.len() as u32).to_le_bytes());
    out[4..4 + compressed.len()].copy_from_slice(&compressed);
    cipher().encrypt(&mut out);
    Ok(out)
}

