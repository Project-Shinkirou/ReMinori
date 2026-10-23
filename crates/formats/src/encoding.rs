pub fn cp932_to_string(bytes: &[u8]) -> String {
    let (cow, had_errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);
    if had_errors {
        log::warn!("CP932 source contains invalid bytes (length={})", bytes.len());
    }
    cow.into_owned()
}
pub fn string_to_cp932(s: &str) -> Vec<u8> {
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.encode(s);
    if had_errors {
        log::warn!("text is not representable in CP932 (UTF-8 bytes={})", s.len());
    }
    cow.into_owned()
}
pub fn string_to_cp932_strict(s: &str) -> Option<Vec<u8>> {
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.encode(s);
    (!had_errors).then(|| cow.into_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptEncoding {
    ShiftJis,
    Gbk,
}

/// Keep the upstream CP932 primary on ties. Compare strict per-line decoding
/// before selecting a translated edition, and never insert replacement text.
pub fn decode_script(bytes: &[u8]) -> anyhow::Result<(ScriptEncoding, std::borrow::Cow<'_, str>)> {
    let invalid_lines = |encoding: &'static encoding_rs::Encoding| {
        bytes.split_inclusive(|byte| *byte == b'\n').filter(|line| {
            let line = line.strip_suffix(b"\r\n").or_else(|| line.strip_suffix(b"\n")).unwrap_or(line);
            encoding.decode_without_bom_handling_and_without_replacement(line).is_none()
        }).count()
    };
    let primary_errors = invalid_lines(encoding_rs::SHIFT_JIS);
    let selected = if primary_errors > 0 && invalid_lines(encoding_rs::GBK) < primary_errors {
        ScriptEncoding::Gbk
    } else {
        ScriptEncoding::ShiftJis
    };
    let encoding = match selected {
        ScriptEncoding::ShiftJis => encoding_rs::SHIFT_JIS,
        ScriptEncoding::Gbk => encoding_rs::GBK,
    };
    let decoded = encoding.decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or_else(|| anyhow::anyhow!("script contains invalid source bytes after strict encoding selection"))?;
    Ok((selected, decoded))
}

#[cfg(test)]
mod script_encoding_tests {
    use super::*;
    #[test]
    fn strict_native_script_selection_preserves_retail_and_translated_text() {
        let japanese = ".message 1   ｱ\r\n.end\r\n";
        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(japanese);
        assert!(!errors);
        let (encoding, text) = decode_script(&bytes).unwrap();
        assert_eq!(encoding, ScriptEncoding::ShiftJis);
        assert_eq!(text, japanese);
        let chinese = ".message 1  姓名 中文剧情\r\n.end\r\n";
        let (bytes, _, errors) = encoding_rs::GBK.encode(chinese);
        assert!(!errors);
        let (encoding, text) = decode_script(&bytes).unwrap();
        assert_eq!(encoding, ScriptEncoding::Gbk);
        assert_eq!(text, chinese);
        assert_eq!(decode_script(b".end\r\n").unwrap().0, ScriptEncoding::ShiftJis);
        assert!(decode_script(b".message 1   \x81\r\n.end\r\n").is_err());
    }
}
