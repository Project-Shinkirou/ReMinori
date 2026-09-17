use eden_formats::{NodeKind, ScriptIr, ScriptStats};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
fn usage() -> ! {
    eprintln!("usage: eden-script <stats|roundtrip> <路径...> [--out FILE]");
    std::process::exit(2);
}
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => {
                let v = it.next().unwrap_or_else(|| usage());
                out = Some(PathBuf::from(v));
            }
            _ if cmd.is_none() => cmd = Some(a),
            _ => paths.push(PathBuf::from(a)),
        }
    }
    match cmd.as_deref() {
        Some("stats") => cmd_stats(&expand(paths)?, out.as_deref()),
        Some("roundtrip") => cmd_roundtrip(&expand(paths)?),
        _ => usage(),
    }
}
fn expand(paths: Vec<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    let roots = if paths.is_empty() {
        eprintln!(
            "未提供路径：请传脚本文件或目录（eden-paz dump-scripts 的产物）"
        );
        usage();
    } else {
        paths
    };
    let mut files = Vec::new();
    for r in roots {
        if r.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&r)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")))
                .collect();
            v.sort();
            files.extend(v);
        } else if r.is_file() {
            files.push(r);
        } else {
            anyhow::bail!("路径不存在: {}", r.display());
        }
    }
    anyhow::ensure!(! files.is_empty(), "未找到脚本文件");
    Ok(files)
}
fn raw_inventory(ir: &ScriptIr) -> BTreeMap<String, (usize, String, String)> {
    let mut m: BTreeMap<String, (usize, String, String)> = BTreeMap::new();
    for n in &ir.nodes {
        if let NodeKind::Raw(r) = &n.kind {
            let e = m.entry(r.tag.clone()).or_insert((0, r.text.clone(), String::new()));
            e.0 += 1;
        }
    }
    m
}
fn cmd_stats(files: &[PathBuf], out: Option<&Path>) -> anyhow::Result<()> {
    let mut agg = ScriptStats::default();
    let mut inv: BTreeMap<String, (usize, String, String)> = BTreeMap::new();
    println!(
        "{:-14} {:>6} {:>6} {:>6} {:>6} {:>7}", "file", "lines", "cmd", "struct", "raw",
        "known%"
    );
    for f in files {
        let ir = ScriptIr::parse(&std::fs::read_to_string(f)?);
        let st = ir.stats();
        println!(
            "{:-14} {:>6} {:>6} {:>6} {:>6} {:>6.1}%", f.file_name().unwrap()
            .to_string_lossy(), st.lines, st.command_lines, st.structured, st.raw, st
            .known_structured_rate().unwrap_or(f64::NAN) * 100.0
        );
        agg.merge(&st);
        for (tag, (n, text, _)) in raw_inventory(&ir) {
            let e = inv
                .entry(tag)
                .or_insert((
                    0,
                    text.clone(),
                    f.file_name().unwrap().to_string_lossy().into_owned(),
                ));
            e.0 += n;
        }
    }
    println!(
        "\n合计: {} 行 / {} 命令行, 结构化 {} ({:.2}%), Raw {} ({:.2}%), 已知命令结构化率 {:.2}%",
        agg.lines, agg.command_lines, agg.structured, agg.structured as f64 / agg
        .command_lines as f64 * 100.0, agg.raw, agg.raw as f64 / agg.command_lines as f64
        * 100.0, agg.known_structured_rate().unwrap_or(f64::NAN) * 100.0
    );
    let mut rows: Vec<_> = inv.into_iter().collect();
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    println!("\n未知/Raw 命令频率表（{} 种）:", rows.len());
    println!("{:>6}  {:<-12} {:<} ", "count", "tag", "example");
    let mut table = String::from("count\ttag\texample\tfile\r\n");
    for (tag, (n, text, file)) in &rows {
        let ex = text.chars().take(60).collect::<String>();
        println!("{n:>6}  {tag:<-12} {ex:<} ({file})");
        table.push_str(&format!("{n}\t{tag}\t{ex}\t{file}\r\n"));
    }
    if let Some(p) = out {
        std::fs::write(p, table)?;
        println!("\n频率表已导出: {}", p.display());
    }
    Ok(())
}
fn cmd_roundtrip(files: &[PathBuf]) -> anyhow::Result<()> {
    let (mut ok, mut bad) = (0usize, 0usize);
    for f in files {
        let text = std::fs::read_to_string(f)?;
        let out = ScriptIr::parse(&text).serialize();
        if out == text {
            ok += 1;
        } else {
            bad += 1;
            eprintln!("MISMATCH: {}", f.display());
            for (i, (a, b)) in text.lines().zip(out.lines()).enumerate() {
                if a != b {
                    eprintln!("  line {}: {:?} != {:?}", i + 1, a, b);
                    break;
                }
            }
        }
    }
    println!("roundtrip: {ok} ok, {bad} mismatch");
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}
