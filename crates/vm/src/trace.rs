use crate::host::Host;
use eden_formats::script::{
    Arg, Effect, Message, Movie, Panel, PlayBgm, PlaySe, Raw, Stage, Transition,
};
use eden_formats::ScriptIr;
use std::collections::HashMap;
pub struct TraceHost {
    pub trace: Vec<String>,
    pub scripts: HashMap<String, ScriptIr>,
    pub unknown_count: usize,
    pub message_done: bool,
    pub voice_active: bool,
    pub panel_mode: i64,
}
impl TraceHost {
    pub fn new() -> Self {
        TraceHost {
            trace: Vec::new(),
            scripts: HashMap::new(),
            unknown_count: 0,
            message_done: true,
            voice_active: false,
            panel_mode: 0,
        }
    }
    pub fn add_script(&mut self, name: &str, ir: ScriptIr) {
        self.scripts.insert(name.to_ascii_lowercase(), ir);
    }
    pub fn take_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.trace)
    }
}
impl Default for TraceHost {
    fn default() -> Self {
        Self::new()
    }
}
fn args(a: &[Arg]) -> String {
    a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}
impl Host for TraceHost {
    fn message(&mut self, m: &Message, _is_read: bool) {
        let voice = m
            .voice
            .as_ref()
            .map(|v| {
                if v.coords.is_empty() {
                    v.name.clone()
                } else {
                    format!(
                        "{}[{}]", v.name, v.coords.iter().map(| c | c.to_string())
                        .collect::< Vec < _ >> ().join(",")
                    )
                }
            })
            .unwrap_or_else(|| "-".into());
        self.trace
            .push(
                format!("message no={} voice={} body={:?}", m.no, voice, m.body_text()),
            );
    }
    fn message_complete(&self) -> bool {
        self.message_done
    }
    fn voice_playing(&self) -> bool {
        self.voice_active
    }
    fn stage(&mut self, s: &Stage) {
        self.trace
            .push(
                format!("stage layer={} file={} x={} y={}", s.layer, s.file, s.x, s.y),
            );
    }
    fn transition(&mut self, t: &Transition) {
        self.trace
            .push(
                format!("transition kind={} mask={} frames={}", t.kind, t.mask, t.frames),
            );
    }
    fn scroll_vh(&mut self, vertical: bool, a: i64, b: i64) {
        self.trace
            .push(format!("scroll_{} a={a} b={b}", if vertical { "v" } else { "h" }));
    }
    fn scroll(&mut self, x: i64, y: i64, n: i64) {
        self.trace.push(format!("scroll x={x} y={y} n={n}"));
    }
    fn end_scroll(&mut self, flag: bool) -> bool {
        self.trace.push(format!("end_scroll flag={flag}"));
        false
    }
    fn shake_screen(&mut self, mode: &str, a: i64, b: i64) {
        self.trace.push(format!("shake_screen mode={mode} a={a} b={b}"));
    }
    fn effect(&mut self, e: &Effect) {
        self.trace
            .push(
                format!(
                    "effect name={} name2={} ints=[{}]", e.name, e.name2.as_ref().map(| a
                    | a.to_string()).unwrap_or_else(|| "-".into()), args(& e.ints)
                ),
            );
    }
    fn play_bgm(&mut self, p: &PlayBgm) {
        self.trace.push(format!("play_bgm file={} rest=[{}]", p.file, args(& p.rest)));
    }
    fn play_se(&mut self, p: &PlaySe) {
        self.trace
            .push(
                format!(
                    "play_se{} file={} vol={} rest=[{}]", if p.se2 { "2" } else { "" }, p
                    .file, p.volume.map(| v | v.to_string()).unwrap_or_else(|| "-"
                    .into()), args(& p.rest)
                ),
            );
    }
    fn play_voice(&mut self, name: &str, rest: &[Arg]) {
        self.trace.push(format!("play_voice name={name} rest=[{}]", args(rest)));
    }
    fn movie(&mut self, m: &Movie) {
        self.trace
            .push(
                format!(
                    "movie id={} file={} w={} h={} skip={} x={} y={}", m.id, m.file, m.w
                    .as_ref().map(| a | a.to_string()).unwrap_or_else(|| "1024".into()),
                    m.h.as_ref().map(| a | a.to_string()).unwrap_or_else(|| "640"
                    .into()), m.skip.as_ref().map(| a | a.to_string()).unwrap_or_else(||
                    "t".into()), m.x.as_ref().map(| a | a.to_string()).unwrap_or_else(||
                    "0".into()), m.y.as_ref().map(| a | a.to_string()).unwrap_or_else(||
                    "0".into()),
                ),
            );
    }
    fn panel(&mut self, p: &Panel) -> bool {
        self.trace.push(format!("panel args=[{}]", args(& p.args)));
        let mode = p.args.first().map_or(0, |a| a.script_int(0));
        let changed = self.panel_mode != mode;
        self.panel_mode = mode;
        changed
    }
    fn pragma(&mut self, name: &str) {
        self.trace.push(format!("pragma {name}"));
    }
    fn end(&mut self) {
        self.trace.push("end".into());
    }
    fn load_script(&mut self, name: &str) -> Option<ScriptIr> {
        self.scripts
            .get(&name.to_ascii_lowercase())
            .map(|ir| ScriptIr {
                nodes: ir.nodes.clone(),
            })
    }
    fn unknown(&mut self, raw: &Raw) {
        self.unknown_count += 1;
        self.trace.push(format!("unknown tag={} line={}", raw.tag, raw.text));
    }
}
