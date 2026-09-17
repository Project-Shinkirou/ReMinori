use anyhow::{bail, ensure, Context};
use eden_formats::{Ani, AniFrame};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Component, Path, PathBuf};
const MANIFEST_NAME: &str = "manifest.ron";
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u16,
    frames: Vec<ManifestFrame>,
}
#[derive(Debug, Serialize, Deserialize)]
struct ManifestFrame {
    image: String,
    label: String,
    flags: u16,
    x: u16,
    y: u16,
}
fn usage() -> ! {
    eprintln!(
        "usage:\n  eden-ani export INPUT.ani [--out DIR]\n  eden-ani pack DIR [--out OUTPUT.ani]"
    );
    std::process::exit(2);
}
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage());
    let input = PathBuf::from(args.next().unwrap_or_else(|| usage()));
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                output = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())));
            }
            _ => usage(),
        }
    }
    match command.as_str() {
        "export" => {
            export_ani(&input, output.as_deref().unwrap_or(&default_export_dir(&input)))
        }
        "pack" => {
            pack_ani(&input, output.as_deref().unwrap_or(&default_pack_path(&input)))
        }
        _ => usage(),
    }
}
fn default_export_dir(input: &Path) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    input.with_file_name(format!("{stem}_ani"))
}
fn default_pack_path(input: &Path) -> PathBuf {
    let name = input.file_name().unwrap_or_default().to_string_lossy();
    let stem = name.strip_suffix("_ani").unwrap_or(&name);
    input.with_file_name(format!("{stem}.ani"))
}
fn export_ani(input: &Path, output: &Path) -> anyhow::Result<()> {
    ensure!(input.is_file(), "ANI 文件不存在: {}", input.display());
    let data = std::fs::read(input)
        .with_context(|| format!("读取失败: {}", input.display()))?;
    let ani = Ani::parse(&data)
        .with_context(|| format!("解析失败: {}", input.display()))?;
    let frames_dir = output.join("frames");
    std::fs::create_dir_all(&frames_dir)
        .with_context(|| format!("无法创建目录: {}", frames_dir.display()))?;
    let mut manifest = Manifest {
        version: 1,
        frames: Vec::with_capacity(ani.frames.len()),
    };
    for (index, frame) in ani.frames.iter().enumerate() {
        let relative = format!("frames/{index:04}.png");
        write_png(&output.join(&relative), frame.w, frame.h, &frame.rgba)?;
        manifest
            .frames
            .push(ManifestFrame {
                image: relative,
                label: frame.label.clone(),
                flags: frame.flags,
                x: frame.x,
                y: frame.y,
            });
    }
    let pretty = ron::ser::PrettyConfig::new().struct_names(true).enumerate_arrays(true);
    let text = ron::ser::to_string_pretty(&manifest, pretty)
        .context("序列化 manifest.ron 失败")?;
    std::fs::write(output.join(MANIFEST_NAME), format!("{text}\n"))?;
    println!("export: {} frames -> {}", manifest.frames.len(), output.display());
    Ok(())
}
fn pack_ani(input: &Path, output: &Path) -> anyhow::Result<()> {
    let manifest_path = input.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("读取失败: {}", manifest_path.display()))?;
    let manifest: Manifest = ron::from_str(&text)
        .with_context(|| format!("清单格式错误: {}", manifest_path.display()))?;
    ensure!(manifest.version == 1, "不支持的 manifest 版本: {}", manifest.version);
    ensure!(
        manifest.frames.len() <= u16::MAX as usize, "帧数超过 ANI 上限: {}",
        manifest.frames.len()
    );
    let mut frames = Vec::with_capacity(manifest.frames.len());
    for (index, item) in manifest.frames.into_iter().enumerate() {
        let relative = safe_relative_path(&item.image)
            .with_context(|| format!("第 {index} 帧 image 路径无效"))?;
        let image_path = input.join(relative);
        let (w, h, rgba) = read_png(&image_path)?;
        let w = u16::try_from(w)
            .with_context(|| {
                format!("第 {index} 帧宽度 {w} 超过 ANI 的 u16 上限")
            })?;
        let h = u16::try_from(h)
            .with_context(|| {
                format!("第 {index} 帧高度 {h} 超过 ANI 的 u16 上限")
            })?;
        frames
            .push(AniFrame {
                label: item.label,
                w,
                h,
                flags: item.flags,
                x: item.x,
                y: item.y,
                rgba,
            });
    }
    let bytes = Ani { frames }
        .encode()
        .with_context(|| format!("无法编码 ANI: {}", output.display()))?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &bytes)
        .with_context(|| format!("写入失败: {}", output.display()))?;
    println!(
        "pack: {} frames, {} bytes -> {}", u16::from_le_bytes([bytes[2], bytes[3]]),
        bytes.len(), output.display()
    );
    Ok(())
}
fn safe_relative_path(value: &str) -> anyhow::Result<PathBuf> {
    let normalized = value.replace('\\', "/");
    let bytes = normalized.as_bytes();
    ensure!(
        ! normalized.starts_with('/') && bytes.get(1) != Some(& b':'),
        "image 必须是清单目录内的相对路径: {value}"
    );
    let path = Path::new(&normalized);
    ensure!(! path.as_os_str().is_empty(), "image 路径为空");
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => clean.push(segment),
            Component::CurDir => {}
            _ => bail!("image 必须是清单目录内的相对路径: {value}"),
        }
    }
    ensure!(! clean.as_os_str().is_empty(), "image 路径为空");
    Ok(clean)
}
fn write_png(path: &Path, w: u16, h: u16, rgba: &[u8]) -> anyhow::Result<()> {
    let file = File::create(path)
        .with_context(|| format!("无法创建 PNG: {}", path.display()))?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        u32::from(w),
        u32::from(h),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
fn read_png(path: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let file = File::open(path)
        .with_context(|| format!("无法打开 PNG: {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("PNG 头无效: {}", path.display()))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .with_context(|| format!("PNG 数据无效: {}", path.display()))?;
    let pixels = &buffer[..info.buffer_size()];
    let count = usize::try_from(info.width)?
        .checked_mul(usize::try_from(info.height)?)
        .context("PNG 尺寸溢出")?;
    let mut rgba = Vec::with_capacity(count.checked_mul(4).context("PNG 尺寸溢出")?);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(pixels),
        png::ColorType::Rgb => {
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Grayscale => {
            for &gray in pixels {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::Indexed => bail!("PNG 调色板展开失败: {}", path.display()),
    }
    ensure!(
        rgba.len() == count * 4, "PNG 像素数据长度不匹配: {}", path.display()
    );
    Ok((info.width, info.height, rgba))
}

