use shinki_emu_build::{write_core_build_manifest, BuildAsset, CoreBuildManifest};
use std::{env, path::PathBuf};

fn main() {
    for name in ["HOST", "TARGET", "PROFILE"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let mut manifest = CoreBuildManifest::new("ReMinori", "rust-cdylib")
        .expect("Cargo must provide the core build target and profile");
    assert!(
        matches!(
            manifest.target.as_str(),
            "x86_64-pc-windows-msvc"
                | "x86_64-unknown-linux-gnu"
                | "aarch64-apple-darwin"
                | "x86_64-apple-darwin"
                | "aarch64-apple-ios"
                | "aarch64-apple-ios-sim"
                | "x86_64-apple-ios"
        ),
        "ReMinori embedded adapter requires a supported Windows, Linux or Apple target"
    );
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("locating the fork source root");
    manifest.licenses = vec![
        BuildAsset { source: root.join("LICENSE"), dest: "licenses/ReMinori/LICENSE".into() },
        BuildAsset { source: root.join("MODIFICATIONS.md"), dest: "licenses/ReMinori/MODIFICATIONS.md".into() },
    ];
    write_core_build_manifest(&manifest).expect("writing ReMinori core-build.json");
}
