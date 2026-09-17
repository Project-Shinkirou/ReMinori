use super::super::Vm;
use crate::host::Host;
use eden_formats::script::{BodyToken, NodeKind};
pub fn exec<H: Host>(vm: &mut Vm<H>) -> crate::Step {
    let NodeKind::Message(m) = &vm.ir.nodes[vm.pc as usize - 1].kind else {
        return crate::Step::Continue;
    };
    let is_read = vm.host.message_is_read(m.no);
    vm.host.message(m, is_read);
    vm.last_message_no = Some(m.no);
    let voice = m
        .voice
        .as_ref()
        .map(|v| {
            let mut s = v.name.clone();
            if !v.coords.is_empty() {
                s.push('[');
                s.push_str(
                    &v.coords.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","),
                );
                s.push(']');
            }
            s
        })
        .unwrap_or_default();
    let speaker = m.speaker.as_ref().map(|s| s.token()).unwrap_or_default();
    vm.backlog
        .push(crate::BacklogEntry {
            script: vm.script_name.clone(),
            line: vm.pc,
            no: m.no,
            voice,
            speaker,
            body: m.body_text(),
            view: crate::MessageView::capture(&vm.system),
        });
    let auto = m.body.iter().any(|t| matches!(t, BodyToken::Ctrl(c) if c == "\\a"));
    let wait_voice = m
        .body
        .iter()
        .any(|t| matches!(t, BodyToken::Ctrl(c) if c == "\\v"));
    vm.state = if auto {
        crate::RunState::AwaitAutoMessage {
            wait_voice,
        }
    } else {
        crate::RunState::AwaitInput
    };
    crate::Step::AwaitInput
}
