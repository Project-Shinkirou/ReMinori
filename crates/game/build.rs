use std::path::Path;
const ICON: &str = "assets/icon.ico";
fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    let icon = Path::new(ICON);
    if !icon.is_file() {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    match icon.to_str() {
        Some(path) => {
            resource.set_icon(path);
        }
        None => {
            println!(
                "cargo:warning={ICON} 路径不是合法 UTF-8，跳过图标嵌入"
            );
            return;
        }
    }
    if let Err(error) = resource.compile() {
        println!(
            "cargo:warning=图标嵌入失败（继续编译无图标版本）: {error}"
        );
    }
}
