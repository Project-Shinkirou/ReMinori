use super::super::Vm;
use crate::host::Host;
use eden_formats::script::NodeKind;
pub fn exec<H: Host>(vm: &mut Vm<H>) -> crate::Step {
    match &vm.ir.nodes[vm.pc as usize - 1].kind {
        NodeKind::Stage(s) => {
            vm.system.insert("stage_BGFilename".into(), s.file.to_string());
            vm.system.insert("stage_BGX".into(), s.x.to_string());
            vm.system.insert("stage_BGY".into(), s.y.to_string());
            vm.system.insert("stage_FGX".into(), "0".into());
            vm.system.insert("stage_FGY".into(), "0".into());
            vm.host.stage(s)
        }
        NodeKind::Transition(t) => {
            vm.system.insert("tr_Method".into(), t.kind.to_string());
            vm.system.insert("tr_Param".into(), t.frames.to_string());
            vm.host.transition(t)
        }
        NodeKind::Vscroll(s) => {
            vm.host.scroll_vh(true, arg_i(s.a.clone()), opt_i(s.b.clone(), 10))
        }
        NodeKind::Hscroll(s) => {
            vm.host.scroll_vh(false, arg_i(s.a.clone()), opt_i(s.b.clone(), 10))
        }
        NodeKind::Scroll(s) => {
            vm.host
                .scroll(
                    arg_i(s.x.clone()),
                    opt_i(s.y.clone(), 0),
                    opt_i(s.n.clone(), 10),
                )
        }
        NodeKind::EndScroll(e) => {
            let flag = e.flag.as_flag().unwrap_or(false);
            let active = vm.host.end_scroll(flag);
            return if flag || !active {
                crate::Step::Continue
            } else {
                vm.state = crate::RunState::WaitScroll;
                crate::Step::AwaitInput
            };
        }
        NodeKind::ShakeScreen(s) => {
            let mode = s.mode.to_string();
            vm.host.shake_screen(&mode, arg_i(s.a.clone()), arg_i(s.b.clone()));
            vm.state = crate::RunState::WaitShake;
            return crate::Step::WaitShake;
        }
        NodeKind::Effect(e) => vm.host.effect(e),
        _ => {}
    }
    crate::Step::Continue
}
fn arg_i(a: eden_formats::script::Arg) -> i64 {
    opt_i(Some(a), 0)
}
fn opt_i(a: Option<eden_formats::script::Arg>, def: i64) -> i64 {
    a.map(|x| x.to_string()).and_then(|t| t.parse::<i64>().ok()).unwrap_or(def)
}
