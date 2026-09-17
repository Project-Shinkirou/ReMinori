pub fn cp932_to_string(bytes: &[u8]) -> String {
    let (cow, had_errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    if had_errors {
        log::warn!(
            "cp932 decode errors in {:02x?} (first 16 bytes)", & bytes.iter().take(16)
            .collect::< Vec < _ >> ()
        );
    }
    cow.into_owned()
}
pub fn string_to_cp932(s: &str) -> Vec<u8> {
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.encode(s);
    if had_errors {
        log::warn!("cp932 encode errors in {s:?}");
    }
    cow.into_owned()
}
pub fn string_to_cp932_strict(s: &str) -> Option<Vec<u8>> {
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.encode(s);
    (!had_errors).then(|| cow.into_owned())
}
