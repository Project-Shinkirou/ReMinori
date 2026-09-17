use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
pub const FIRST_USER_SLOT: usize = 10;
pub const LAST_USER_SLOT: usize = 99;
pub fn suggested_slot(file_name: &str) -> usize {
    parse_slot(file_name).unwrap_or(FIRST_USER_SLOT)
}
fn parse_slot(file_name: &str) -> Option<usize> {
    let lower = file_name.to_ascii_lowercase();
    let digits = lower.strip_prefix("eden")?.strip_suffix(".sav")?;
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let slot = digits.parse().ok()?;
    (FIRST_USER_SLOT..=LAST_USER_SLOT).contains(&slot).then_some(slot)
}
pub fn validate_save(bytes: &[u8]) -> Result<String, String> {
    eden_formats::read_save_file(bytes)
        .map(|file| file.header.comment)
        .map_err(|error| format!("不是有效的 eden 存档: {error}"))
}
pub fn placeholder_thumbnail_png() -> Result<Vec<u8>, String> {
    const WIDTH: u32 = 128;
    const HEIGHT: u32 = 80;
    let mut rgba = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[224, 224, 224, 255]);
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer.write_image_data(&rgba).map_err(|error| error.to_string())?;
    }
    Ok(output)
}
pub fn build_saves_zip(mut entries: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    entries.retain(|(name, _)| safe_save_file_name(name));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if !entries.iter().any(|(name, _)| name.ends_with(".sav")) {
        return Err("当前没有可导出的存档".into());
    }
    let cursor = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o600);
    for (name, bytes) in entries {
        archive
            .start_file(format!("saves/{name}"), options)
            .map_err(|error| error.to_string())?;
        archive.write_all(&bytes).map_err(|error| error.to_string())?;
    }
    archive.finish().map(Cursor::into_inner).map_err(|error| error.to_string())
}
fn safe_save_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':', '\0'])
        && (name.to_ascii_lowercase().ends_with(".sav")
            || name.to_ascii_lowercase().ends_with(".png"))
}

