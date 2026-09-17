use super::super::Vm;
use crate::host::Host;
use eden_formats::script::NodeKind;
pub fn exec<H: Host>(vm: &mut Vm<H>) -> crate::Step {
    match &vm.ir.nodes[vm.pc as usize - 1].kind {
        NodeKind::Goto(g) => {
            let target = g.target.clone();
            if vm.jump_to_label(&target) {
                crate::Step::Jumped
            } else {
                crate::Step::Continue
            }
        }
        NodeKind::If(i) => {
            let hit = vm.eval_if(i);
            let target = i.target.clone();
            if hit && vm.jump_to_label(&target) {
                crate::Step::Jumped
            } else {
                crate::Step::Continue
            }
        }
        NodeKind::Chain(c) => {
            let next = c.script.clone();
            match vm.host.load_script(&next) {
                Some(ir) => {
                    vm.locals.clear();
                    vm.last_message_no = None;
                    let labels = super::super::label_map(&ir);
                    vm.ir = ir;
                    vm.labels = labels;
                    vm.pc = 1;
                    vm.script_name = next.clone();
                    vm.state = crate::RunState::Running;
                    crate::Step::Chain { next }
                }
                None => {
                    log::warn!(
                        "chain 目标缺失: {next}（{}:{}）", vm.script_name, vm.pc
                    );
                    vm.state = crate::RunState::Ended;
                    crate::Step::Ended
                }
            }
        }
        NodeKind::End => {
            vm.host.end();
            vm.state = crate::RunState::Ended;
            crate::Step::Ended
        }
        _ => crate::Step::Continue,
    }
}
