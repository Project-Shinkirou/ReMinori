use super::super::Vm;
use crate::host::Host;
use eden_formats::script::NodeKind;
pub fn exec<H: Host>(vm: &mut Vm<H>) -> crate::Step {
    match &vm.ir.nodes[vm.pc as usize - 1].kind {
        NodeKind::PlayBgm(p) => {
            if p.file == eden_formats::script::Arg::Star {
                vm.system.remove("bgm_Filename");
            } else {
                vm.system.insert("bgm_Filename".into(), p.file.to_string());
            }
            vm.system.insert("bgm_Volume".into(), p.volume().to_string());
            vm.host.play_bgm(p)
        }
        NodeKind::PlaySe(p) => {
            let key = if p.se2 { "se2_Filename" } else { "se1_Filename" };
            if p.file == eden_formats::script::Arg::Star {
                vm.system.remove(key);
            } else if p.looping() {
                let mut name = p.file.to_string();
                if let Some(v) = p.volume {
                    name.push('[');
                    name.push_str(&v.to_string());
                    name.push(']');
                }
                vm.system.insert(key.into(), name);
            }
            vm.host.play_se(p)
        }
        NodeKind::PlayVoice(p) => {
            let name = p.name.to_string();
            vm.host.play_voice(&name, &p.rest)
        }
        NodeKind::Movie(m) => {
            vm.host.movie(m);
            vm.state = crate::RunState::AwaitInput;
            return crate::Step::AwaitInput;
        }
        _ => {}
    }
    crate::Step::Continue
}
