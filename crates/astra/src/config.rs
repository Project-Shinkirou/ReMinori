use crate::{bytes, Failure};
use astra_emu_abi as abi;
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

struct Schema([abi::ConfigField; 3]);
// All nested schema spans point to immutable library-lifetime constants.
unsafe impl Send for Schema {}
unsafe impl Sync for Schema {}
static SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
    let string = |id, label, description, flags| abi::ConfigField {
        id: bytes(id),
        label: bytes(label),
        group: bytes("ReMinori"),
        default_value: abi::ConfigValue {
            kind: abi::VALUE_STRING,
            reserved: 0,
            integer: 0,
            number: 0.0,
            text: abi::Bytes::EMPTY,
        },
        integer_min: 0,
        integer_max: 0,
        number_min: 0.0,
        number_max: 0.0,
        max_bytes: 4096,
        flags,
        choices: std::ptr::null(),
        choice_count: 0,
        description: bytes(description),
        choice_labels: std::ptr::null(),
        step: 0.0,
        unit: abi::Bytes::EMPTY,
    };
    let mut null_audio = string(
        "diagnostic_null_audio",
        "Diagnostic silent audio",
        "Explicit diagnostics only; normal sessions require native audio output.",
        0,
    );
    null_audio.default_value.kind = abi::VALUE_BOOL;
    null_audio.max_bytes = 0;
    Schema([
            string("profile_path", "Archive profile", "keys.v2 key.toml; empty selects the upstream retail CD-ROM parameters.", abi::CONFIG_FLAG_FILE_PATH | abi::CONFIG_FLAG_GAME_RELATIVE),
            string("save_directory", "Isolated save directory", "Required absolute directory outside the game; all native saves, settings and movie cache stay here.", abi::CONFIG_FLAG_DIRECTORY_PATH),
            null_audio,
        ])
});
pub fn fields() -> &'static [abi::ConfigField] {
    &SCHEMA.0
}
pub struct Configuration {
    pub game: PathBuf,
    pub profile: Option<PathBuf>,
    pub saves: PathBuf,
    pub null_audio: bool,
}
pub unsafe fn parse(request: &abi::OpenRequest) -> Result<Configuration, Failure> {
    let entries = unsafe {
        abi::borrowed_array(
            request.configuration,
            request.configuration_count,
            abi::MAX_CONFIG_FIELDS,
        )
    }
    .map_err(Failure::status)?;
    unsafe { abi::validate_config(fields(), entries) }
        .map_err(|_| Failure::invalid("invalid typed core configuration"))?;
    let path = unsafe { request.game_path.text(32768) }.map_err(Failure::status)?;
    let path = Path::new(path);
    let game = (if path.is_dir() {
        path
    } else {
        path.parent()
            .ok_or_else(|| Failure::invalid("missing game directory"))?
    })
    .canonicalize()
    .map_err(|_| Failure::invalid("game directory does not exist"))?;
    let mut profile = None;
    let mut saves = PathBuf::new();
    let mut null_audio = false;
    for entry in entries {
        match unsafe { entry.id.text(128) }.map_err(Failure::status)? {
            "profile_path" => {
                let text = unsafe { entry.value.text.text(4096) }.map_err(Failure::status)?;
                if !text.is_empty() {
                    let path = PathBuf::from(text);
                    let path = if path.is_absolute() {
                        path
                    } else {
                        game.join(path)
                    };
                    profile = Some(
                        path.canonicalize()
                            .map_err(|_| Failure::invalid("archive profile does not exist"))?,
                    );
                }
            }
            "save_directory" => {
                saves =
                    PathBuf::from(unsafe { entry.value.text.text(4096) }.map_err(Failure::status)?)
            }
            "diagnostic_null_audio" => null_audio = entry.value.integer == 1,
            _ => return Err(Failure::invalid("unknown core configuration")),
        }
    }
    if !saves.is_absolute() {
        return Err(Failure::invalid(
            "save_directory must be an explicit absolute directory",
        ));
    }
    // Resolve the existing ancestor before creating anything. Reject junctions
    // back into the game and game ancestors as well as direct game paths.
    let mut ancestor = saves.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| Failure::invalid("invalid save directory"))?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Failure::invalid("invalid save directory"))?;
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|_| Failure::invalid("cannot resolve save directory"))?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    if resolved.starts_with(&game) || game.starts_with(&resolved) {
        return Err(Failure::invalid(
            "save directory must be isolated from the game",
        ));
    }
    Ok(Configuration {
        game,
        profile,
        saves: resolved,
        null_audio,
    })
}
