use eden_formats::{read_sav, ScriptIr};
use eden_vm::{Step, TraceHost, Vm};
use std::collections::BTreeMap;
use std::path::PathBuf;
fn usage() -> ! {
    eprintln!(
        "usage: eden-vm-trace <script.sc> --corpus DIR [--line N] [--sav FILE] [--raw-backlog K]"
    );
    std::process::exit(2);
}
fn main() -> anyhow::Result<()> {
    let mut script: Option<String> = None;
    let mut line: Option<u32> = None;
    let mut sav: Option<PathBuf> = None;
    let mut corpus: Option<PathBuf> = None;
    let mut raw_backlog = 0usize;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0usize;
    while i < args.len() {
        let arg = |i: &mut usize| -> String {
            *i += 1;
            args.get(*i).cloned().unwrap_or_else(|| usage())
        };
        match args[i].as_str() {
            "--line" => line = Some(arg(&mut i).parse().unwrap_or_else(|_| usage())),
            "--sav" => sav = Some(PathBuf::from(arg(&mut i))),
            "--corpus" => corpus = Some(PathBuf::from(arg(&mut i))),
            "--raw-backlog" => raw_backlog = arg(&mut i).parse().unwrap_or(3),
            _ if script.is_none() => script = Some(args[i].clone()),
            _ => usage(),
        }
        i += 1;
    }
    let script = match script {
        Some(s) => s,
        None => usage(),
    };
    let corpus = match corpus {
        Some(c) => c,
        None => {
            eprintln!("缺少 --corpus DIR（eden-paz dump-scripts 产物目录）");
            usage();
        }
    };
    let key = script.to_ascii_lowercase();
    let mut host = TraceHost::new();
    for e in std::fs::read_dir(&corpus)? {
        let p = e?.path();
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("txt")) {
            let name = format!(
                "{}.sc", p.file_stem().unwrap().to_string_lossy().to_ascii_lowercase()
            );
            let text = std::fs::read_to_string(&p)?;
            host.add_script(&name, ScriptIr::parse(&text));
        }
    }
    let ir = host
        .scripts
        .get(&key)
        .map(|i| ScriptIr { nodes: i.nodes.clone() })
        .ok_or_else(|| anyhow::anyhow!("语料缺 {script}（{key}）"))?;
    let mut vm = Vm::new(host, &key, ir);
    let mut steps = 0usize;
    loop {
        if let Some(n) = line {
            if vm.pc() == n && matches!(vm.state(), eden_vm::RunState::AwaitInput) {
                break;
            }
            if vm.pc() > n {
                anyhow::bail!(
                    "第 {n} 行不是等待输入的消息行（越过时状态 {:?}）",
                    vm.state()
                );
            }
        }
        steps += 1;
        if steps > 500_000 {
            anyhow::bail!("步数超限，疑似死循环");
        }
        match vm.step() {
            Step::Continue | Step::Chain { .. } | Step::Jumped => {}
            Step::Wait { .. } => {
                vm.pump(u32::MAX);
            }
            Step::AwaitInput => {
                if let Some(n) = line {
                    if vm.pc() == n {
                        break;
                    }
                }
                vm.advance_input()
            }
            Step::WaitShake => vm.advance_shake(),
            Step::Ended => break,
        }
    }
    let snap = vm.system_snapshot();
    println!(
        "checkpoint: script={} line={} steps={} backlog={}", vm.script_name(), vm.pc(),
        steps, vm.backlog().len()
    );
    for e in vm.backlog().iter().rev().take(raw_backlog) {
        println!(
            "  backlog: no={} line={} voice={:?} speaker={:?} body={:?}", e.no, e.line, e
            .voice, e.speaker, e.body
        );
    }
    match sav {
        None => Ok(()),
        Some(path) => {
            let data = read_sav(&std::fs::read(&path)?)?;
            let orig: BTreeMap<String, String> = data.variables.into_iter().collect();
            println!("\n变量区 diff（VM vs {}）：", path.display());
            let mut bad = 0usize;
            for k in orig
                .keys()
                .chain(snap.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let (a, b) = (orig.get(k), snap.get(k));
                match (a, b) {
                    (Some(x), Some(y)) if x == y => println!("  MATCH {k:<18} {x}"),
                    (Some(x), Some(y)) => {
                        println!("  DIFF  {k:<18} sav={x:?} vm={y:?}");
                        bad += 1;
                    }
                    (Some(x), None) => {
                        println!("  ONLY-SAV {k:<16} {x}");
                        bad += 1;
                    }
                    (None, Some(y)) => {
                        println!("  ONLY-VM {k:<17} {y}");
                        bad += 1;
                    }
                    _ => unreachable!(),
                }
            }
            println!("结果: {} 字段, {} 不一致", orig.len().max(snap.len()), bad);
            if bad > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}
