use super::super::Vm;
use crate::host::Host;
use eden_formats::script::NodeKind;
pub fn exec<H: Host>(vm: &mut Vm<H>) -> crate::Step {
    match &vm.ir.nodes[vm.pc as usize - 1].kind {
        NodeKind::SetGlobal(a) => {
            let (var, val) = (a.var.clone(), a.value.clone());
            vm.set_var(true, &var, &val)
        }
        NodeKind::Set(a) => {
            let (var, val) = (a.var.clone(), a.value.clone());
            vm.set_var(false, &var, &val)
        }
        NodeKind::Panel(p) => {
            let changed = vm.host.panel(p);
            if let Some(mode) = p.args.first() {
                vm.system.insert("panel_Mode".into(), mode.to_string());
            }
            if changed {
                let speed = p.args.get(1).map_or(-1, |a| a.script_int(-1));
                let speed = if speed <= 0 { 16 } else { speed as u32 };
                let frames = 256 / speed;
                if frames > 0 {
                    vm.state = crate::RunState::WaitFrames(frames);
                    return crate::Step::Wait { frames };
                }
            }
        }
        NodeKind::Wait(w) => {
            let frames = w.frames.as_i64().unwrap_or(0).max(0) as u32;
            vm.state = crate::RunState::WaitFrames(frames);
            return crate::Step::Wait { frames };
        }
        NodeKind::Pragma(p) => {
            let name = p.name.clone();
            vm.pragmas.insert(name.clone());
            if let Some(base) = name.strip_prefix("enable_") {
                vm.pragmas.remove(&format!("disable_{base}"));
            }
            if name == "monologue" {
                let target = vm
                    .ir
                    .nodes[vm.pc as usize..]
                    .iter()
                    .find_map(|n| match &n.kind {
                        NodeKind::Pragma(q) if q.name == "monologue_end" => Some(n.line),
                        _ => None,
                    });
                if let Some(line) = target {
                    vm.pc = line;
                    return crate::Step::Jumped;
                }
            }
            vm.host.pragma(&name);
            match name.as_str() {
                "clear_global_var" => vm.globals.clear(),
                "clear_local_var" => vm.locals.clear(),
                _ => {}
            }
        }
        NodeKind::Raw(r) => vm.host.unknown(r),
        _ => {}
    }
    crate::Step::Continue
}
