use serde::{Deserialize, Serialize};
use std::io::Read;
use std::io::Write;
const ZLIB_MAGIC: &[u8] = &[0x78, 0x9C];
const CONTAINER_MAGIC: &[u8] = b";\n!\xAA";
const SIGNATURE: &[u8] = b"eden 1.00";
const PORT_SIGNATURE: &[u8] = b"eden port 1.00";
pub const BACKLOG_FIELD_ORDER: &[&str] = &[
    "LC",
    "L0",
    "L1",
    "L2",
    "L3",
    "CS1",
    "CS2",
    "CS3",
    "CF1",
    "CF2",
    "CF3",
    "CP1",
    "CP2",
    "CT1",
    "CT2",
    "CT3",
    "CT4",
    "CT5",
    "CT6",
    "CT7",
    "Frm",
    "FrP",
    "CO0",
    "CO1",
    "CO2",
    "Cs0",
    "Cs1",
    "Cs2",
    "bgH",
    "bgS",
    "bgV",
    "stH",
    "stS",
    "stV",
    "CB1",
    "CB1v",
    "CE1",
    "CE2",
    "Ce1",
    "Ce2",
    "Ce3",
    "Ce4",
    "Ce5",
    "Ct1",
    "Ct2",
    "Ct3",
];
#[derive(Debug)]
pub enum SaveError {
    BadContainer(&'static str),
    Zlib,
    Section(&'static str),
}
impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::BadContainer(w) => write!(f, "容器异常: {w}"),
            SaveError::Zlib => write!(f, "zlib 解压失败"),
            SaveError::Section(w) => write!(f, "段落缺失: {w}"),
        }
    }
}
impl std::error::Error for SaveError {}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveData {
    pub variables: Vec<(String, String)>,
    pub backlog: Vec<BacklogEntryRaw>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BacklogEntryRaw {
    pub fields: Vec<(String, String)>,
}
impl BacklogEntryRaw {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}
pub fn read_sav(bytes: &[u8]) -> Result<SaveData, SaveError> {
    if !bytes.starts_with(b";\n!") {
        return Err(SaveError::BadContainer("缺 ;\\n! 前缀"));
    }
    let z = bytes
        .windows(2)
        .position(|w| w == ZLIB_MAGIC)
        .ok_or(SaveError::BadContainer("未找到 zlib 流"))?;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[z..])
        .read_to_end(&mut raw)
        .map_err(|_| SaveError::Zlib)?;
    if is_port_save(bytes) {
        let text = std::str::from_utf8(&raw)
            .map_err(|_| SaveError::BadContainer("端口存档体不是 UTF-8"))?;
        parse_body(text)
    } else {
        parse_body(&crate::encoding::cp932_to_string(&raw))
    }
}
pub fn parse_body(text: &str) -> Result<SaveData, SaveError> {
    let mut variables = Vec::new();
    let mut backlog = Vec::new();
    let mut section = 0u8;
    let mut cur: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        match line {
            "<begin variables>" => section = 1,
            "<end variables>" => section = 0,
            "<<begin backlog>>" => section = 2,
            "<<end backlog>>" => {
                if !cur.is_empty() {
                    backlog
                        .push(BacklogEntryRaw {
                            fields: std::mem::take(&mut cur),
                        });
                }
                section = 0;
            }
            "ZZ" => {
                backlog
                    .push(BacklogEntryRaw {
                        fields: std::mem::take(&mut cur),
                    })
            }
            _ if section == 1 => {
                if let Some(rest) = line.strip_prefix('!') {
                    if let Some(i) = rest.find('\t') {
                        variables
                            .push((rest[..i].to_string(), rest[i + 1..].to_string()));
                    }
                }
            }
            _ if section == 2 => {
                if let Some(i) = line.find('\t') {
                    cur.push((line[..i].to_string(), line[i + 1..].to_string()));
                } else if !line.is_empty() {
                    cur.push((line.to_string(), String::new()));
                }
            }
            _ => {}
        }
    }
    if variables.is_empty() {
        return Err(SaveError::Section("<begin variables>"));
    }
    Ok(SaveData { variables, backlog })
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveHeader {
    pub comment: String,
    pub route: [u8; 4],
}
impl Default for SaveHeader {
    fn default() -> Self {
        SaveHeader {
            comment: String::new(),
            route: [0; 4],
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveFile {
    pub header: SaveHeader,
    pub body: SaveData,
}
pub fn read_save_file(bytes: &[u8]) -> Result<SaveFile, SaveError> {
    let after_magic = bytes
        .strip_prefix(CONTAINER_MAGIC)
        .ok_or(SaveError::BadContainer("缺 ;\\n!\\xAA 前缀"))?;
    let (rest, utf8) = if let Some(rest) = after_magic.strip_prefix(PORT_SIGNATURE) {
        (rest, true)
    } else if let Some(rest) = after_magic.strip_prefix(SIGNATURE) {
        (rest, false)
    } else {
        return Err(SaveError::BadContainer("缺已知存档签名"));
    };
    let rest = rest
        .strip_prefix(&[0])
        .ok_or(SaveError::BadContainer("缺签名后 NUL"))?;
    let cend = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or(SaveError::BadContainer("注释未终止"))?;
    let comment = if utf8 {
        std::str::from_utf8(&rest[..cend])
            .map_err(|_| SaveError::BadContainer("端口存档注释不是 UTF-8"))?
            .to_string()
    } else {
        crate::encoding::cp932_to_string(&rest[..cend])
    };
    let rest = &rest[cend + 1..];
    if rest.len() < 4 {
        return Err(SaveError::BadContainer("缺 route 4 字节"));
    }
    let header = SaveHeader {
        comment,
        route: [rest[0], rest[1], rest[2], rest[3]],
    };
    Ok(SaveFile {
        header,
        body: read_sav(bytes)?,
    })
}
pub fn decompressed_body(bytes: &[u8]) -> Result<String, SaveError> {
    let z = bytes
        .windows(2)
        .position(|w| w == ZLIB_MAGIC)
        .ok_or(SaveError::BadContainer("未找到 zlib 流"))?;
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[z..])
        .read_to_end(&mut raw)
        .map_err(|_| SaveError::Zlib)?;
    if is_port_save(bytes) {
        String::from_utf8(raw)
            .map_err(|_| SaveError::BadContainer("端口存档体不是 UTF-8"))
    } else {
        Ok(crate::encoding::cp932_to_string(&raw))
    }
}
pub fn write_sav(file: &SaveFile) -> Vec<u8> {
    let body = crate::encoding::string_to_cp932(&write_body(&file.body));
    write_container(
        file,
        SIGNATURE,
        crate::encoding::string_to_cp932(&file.header.comment),
        body,
    )
}
pub fn write_port_sav(file: &SaveFile) -> Vec<u8> {
    write_container(
        file,
        PORT_SIGNATURE,
        file.header.comment.as_bytes().to_vec(),
        write_body(&file.body).into_bytes(),
    )
}
fn write_container(
    file: &SaveFile,
    signature: &[u8],
    comment: Vec<u8>,
    body: Vec<u8>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() / 2 + 64);
    out.extend_from_slice(CONTAINER_MAGIC);
    out.extend_from_slice(signature);
    out.push(0);
    out.extend_from_slice(&comment);
    out.push(0);
    out.extend_from_slice(&file.header.route);
    let mut enc = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
    enc.write_all(&body).expect("zlib 编码不会失败");
    enc.finish().expect("zlib finish 不会失败")
}
fn is_port_save(bytes: &[u8]) -> bool {
    bytes
        .strip_prefix(CONTAINER_MAGIC)
        .is_some_and(|rest| rest.starts_with(PORT_SIGNATURE))
}
pub fn write_body(data: &SaveData) -> String {
    let mut s = String::new();
    s.push_str("<begin variables>\n");
    for (k, v) in &data.variables {
        s.push('!');
        s.push_str(k);
        s.push('\t');
        s.push_str(v);
        s.push('\n');
    }
    s.push_str("<end variables>\n");
    s.push_str("<<begin backlog>>\n");
    for e in &data.backlog {
        for key in BACKLOG_FIELD_ORDER {
            let v = e.get(key).unwrap_or("");
            s.push_str(key);
            s.push('\t');
            s.push_str(v);
            s.push('\n');
        }
        s.push_str("ZZ\n");
    }
    s.push_str("<<end backlog>>\n");
    s
}

#[cfg(test)]
mod unicode_save_tests {
    use super::*;
    #[test]
    fn native_port_save_preserves_translated_dialogue_and_comment() {
        let fields = BACKLOG_FIELD_ORDER.iter().map(|key| {
            ((*key).to_owned(), if *key == "CT1" { "中文剧情".to_owned() } else { String::new() })
        }).collect();
        let file = SaveFile {
            header: SaveHeader { comment: "中文存档".into(), route: [0; 4] },
            body: SaveData {
                variables: vec![("public_fixture".into(), "中文文本".into())],
                backlog: vec![BacklogEntryRaw { fields }],
            },
        };
        assert_eq!(read_save_file(&write_port_sav(&file)).unwrap(), file);
    }
}
