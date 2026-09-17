use eden_formats::{decode, Ani, PazArchive, ResourceKind};
use std::path::PathBuf;
struct Common {
    slot: usize,
    path: PathBuf,
    xor_key: u8,
}
fn usage() -> ! {
    eprintln!(
        "usage: eden-paz <list|verify|extract> <slot> <paz路径> [glob ...] [--out DIR] [--raw] [--xor-key hex]"
    );
    eprintln!("       eden-paz dump-scripts <scr.paz路径> --out DIR [--xor-key hex]");
    eprintln!(
        "slot = WinMain 归档序号：0=scr 1/2=bg 3=st 4=sys 5=bgm 6=voice 7/8=se 9=mov"
    );
    eprintln!("extract / dump-scripts 必须提供 --out DIR");
    std::process::exit(2);
}
fn parse_args(
    args: &[String],
    need_slot: bool,
) -> (Option<usize>, Option<String>, Vec<String>, Option<String>, bool, u8) {
    let mut positionals = Vec::new();
    let mut xor_key = 0xAAu8;
    let mut out = None;
    let mut raw = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--xor-key" => {
                let v = it.next().unwrap_or_else(|| usage());
                let h = v.trim_start_matches("0x");
                xor_key = u8::from_str_radix(h, 16)
                    .unwrap_or_else(|_| {
                        eprintln!("bad --xor-key {v}");
                        usage()
                    });
            }
            "--out" => out = Some(it.next().unwrap_or_else(|| usage()).clone()),
            "--raw" => raw = true,
            _ => positionals.push(a.clone()),
        }
    }
    let (mut slot, mut path, mut globs) = (None, None, Vec::new());
    for p in &positionals {
        if slot.is_none() && need_slot && !p.is_empty()
            && p.chars().all(|c| c.is_ascii_digit())
        {
            slot = Some(p.parse().unwrap_or_else(|_| usage()));
        } else if path.is_none()
            && (p.to_ascii_lowercase().ends_with(".paz") || p.contains('\\')
                || p.contains('/'))
        {
            path = Some(p.clone());
        } else {
            globs.push(p.clone());
        }
    }
    if need_slot && slot.is_none() {
        usage();
    }
    (slot, path, globs, out, raw, xor_key)
}
fn common_for(slot: usize, path: Option<String>, xor_key: u8) -> Common {
    let group = PazArchive::group_name(slot);
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!(
                "缺少 paz 路径：slot {slot} = {group}，请传 <{group}.paz> 的路径"
            );
            usage();
        }
    };
    Common { slot, path, xor_key }
}
fn open(c: &Common) -> anyhow::Result<PazArchive> {
    let pz = if c.slot == 9 {
        PazArchive::open_main_with_material(&c.path, c.slot, c.xor_key)?
    } else {
        PazArchive::open_main(&c.path, c.slot, c.xor_key)?
    };
    println!(
        "{}: prefix={:?} entries={}", c.path.display(), pz.prefix, pz.entries.len()
    );
    Ok(pz)
}
fn entry_bytes(
    pz: &PazArchive,
    name: &str,
    size: u32,
    raw: bool,
) -> anyhow::Result<Vec<u8>> {
    let mut data = pz.read(name)?;
    if !raw {
        if let Some(kind) = ResourceKind::from_name(name) {
            decode(name, size, kind, &mut data);
        }
    }
    Ok(data)
}
fn magic_of(name: &str, data: &[u8]) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "PNG"
    } else if lower.ends_with(".sc") && data.starts_with(b".") {
        "script-text"
    } else if data.starts_with(b"OggS") {
        "OggS"
    } else if data.starts_with(b"RIFF") {
        "RIFF"
    } else if lower.ends_with(".ani") && Ani::is_ani(data) {
        "ani"
    } else {
        "???"
    }
}
fn glob_match(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (
        pat.to_ascii_lowercase().chars().collect(),
        name.to_ascii_lowercase().chars().collect(),
    );
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                (1..=p.len()).any(|i| go(&p[i..], n))
                    || (!n.is_empty() && go(p, &n[1..]))
            }
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    go(&p, &n)
}
fn sanitize_rel(name: &str) -> PathBuf {
    let mut pb = PathBuf::new();
    for seg in name.replace('\\', "/").split('/') {
        if !seg.is_empty() && seg != "." && seg != ".." {
            pb.push(seg);
        }
    }
    pb
}
fn cmd_list(c: &Common) -> anyhow::Result<()> {
    let pz = open(c)?;
    println!(
        "{:-24} {:>10} {:>10} {:>10} {:>10} {:>6}", "name", "off", "size", "stored",
        "aligned", "method"
    );
    for e in &pz.entries {
        println!(
            "{:-24} {:>10x} {:>10} {:>10} {:>10} {:>6}", e.name, e.offset, e.size, e
            .stored, e.aligned, e.method
        );
    }
    Ok(())
}
fn cmd_verify(c: &Common) -> anyhow::Result<()> {
    let pz = open(c)?;
    let (mut ok, mut unknown) = (0usize, 0usize);
    for e in &pz.entries {
        let data = entry_bytes(&pz, &e.name, e.size, false)?;
        if magic_of(&e.name, &data) != "???" {
            ok += 1;
        } else {
            unknown += 1;
            println!(
                "  ?? {}: {}", e.name, data.iter().take(8).map(| b | format!("{b:02x}"))
                .collect::< String > ()
            );
        }
    }
    println!("verify: {ok} ok, {unknown} unknown");
    Ok(())
}
fn cmd_extract(
    c: &Common,
    globs: &[String],
    out: Option<String>,
    raw: bool,
) -> anyhow::Result<()> {
    let pz = open(c)?;
    let out_dir = match out {
        Some(o) => PathBuf::from(o),
        None => {
            eprintln!("extract 缺少 --out DIR");
            usage();
        }
    };
    let (mut n, mut bytes) = (0usize, 0u64);
    for e in &pz.entries {
        if !globs.is_empty() && !globs.iter().any(|g| glob_match(g, &e.name)) {
            continue;
        }
        let data = entry_bytes(&pz, &e.name, e.size, raw)?;
        let dest = out_dir.join(sanitize_rel(&e.name));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &data)?;
        n += 1;
        bytes += data.len() as u64;
    }
    println!("extract: {n} files, {} bytes -> {}", human(bytes), out_dir.display());
    Ok(())
}
fn cmd_dump_scripts(c: &Common, out: Option<String>) -> anyhow::Result<()> {
    let pz = open(c)?;
    let out_dir = match out {
        Some(o) => PathBuf::from(o),
        None => {
            eprintln!("dump-scripts 缺少 --out DIR");
            usage();
        }
    };
    std::fs::create_dir_all(&out_dir)?;
    let mut ok = 0usize;
    for e in &pz.entries {
        let mut data = pz.read(&e.name)?;
        decode(&e.name, e.size, ResourceKind::Script, &mut data);
        if !data.starts_with(b".") {
            eprintln!(
                "  ?? {} 非脚本文本: {}", e.name, data.iter().take(8).map(| b |
                format!("{b:02x}")).collect::< String > ()
            );
            continue;
        }
        let text = eden_formats::encoding::cp932_to_string(&data);
        let dest = out_dir.join(e.name.to_ascii_lowercase().replace(".sc", ".txt"));
        std::fs::write(dest, text)?;
        ok += 1;
    }
    println!("dump-scripts: {}/{} -> {}", ok, pz.entries.len(), out_dir.display());
    Ok(())
}
fn human(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.2} GiB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.2} MiB", b as f64 / (1u64 << 20) as f64)
    } else if b >= 1 << 10 {
        format!("{:.2} KiB", b as f64 / (1u64 << 10) as f64)
    } else {
        format!("{b} B")
    }
}
fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = if args.is_empty() { usage() } else { args.remove(0) };
    match cmd.as_str() {
        "list" | "verify" | "extract" => {
            let (slot, path, globs, out, raw, xor_key) = parse_args(&args, true);
            let c = common_for(slot.unwrap(), path, xor_key);
            match cmd.as_str() {
                "list" => cmd_list(&c),
                "verify" => cmd_verify(&c),
                _ => cmd_extract(&c, &globs, out, raw),
            }
        }
        "dump-scripts" => {
            let (_, path, _, out, _, xor_key) = parse_args(&args, false);
            cmd_dump_scripts(&common_for(0, path, xor_key), out)
        }
        _ => usage(),
    }
}
