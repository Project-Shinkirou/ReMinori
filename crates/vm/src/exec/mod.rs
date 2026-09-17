use crate::host::Host;
use eden_formats::script::{If, NodeKind, ScriptIr};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
pub mod handlers;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Continue,
    Wait { frames: u32 },
    AwaitInput,
    WaitShake,
    Chain { next: String },
    Ended,
    Jumped,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    Running,
    WaitFrames(u32),
    AwaitInput,
    AwaitAutoMessage { wait_voice: bool },
    WaitShake,
    WaitScroll,
    Ended,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageView {
    pub font_color: String,
    pub font_size: String,
    pub ruby_size: String,
    pub panel_mode: String,
    pub bg: String,
    pub bg_x: String,
    pub bg_y: String,
    pub panel_fade: String,
    pub bgm: String,
    pub bgm_volume: String,
    pub se1: String,
    pub se2: String,
    pub eff_count: String,
    pub eff_speed: String,
    pub eff_param: String,
    pub frame_param: String,
    pub bg_h: String,
    pub bg_s: String,
    pub bg_v: String,
    pub st_h: String,
    pub st_s: String,
    pub st_v: String,
    pub tr_param: String,
}
impl MessageView {
    fn from_raw(e: &eden_formats::save::BacklogEntryRaw) -> Self {
        let g = |k: &str| e.get(k).unwrap_or("").to_string();
        MessageView {
            font_color: g("CF1"),
            font_size: g("CF2"),
            ruby_size: g("CF3"),
            panel_mode: g("CP1"),
            bg: g("CT2"),
            bg_x: g("CT3"),
            bg_y: g("CT4"),
            panel_fade: g("CT5"),
            bgm: g("CB1"),
            bgm_volume: g("CB1v"),
            se1: g("CE1"),
            se2: g("CE2"),
            eff_count: g("Ce3"),
            eff_speed: g("Ce4"),
            eff_param: g("Ce5"),
            frame_param: g("FrP"),
            bg_h: g("bgH"),
            bg_s: g("bgS"),
            bg_v: g("bgV"),
            st_h: g("stH"),
            st_s: g("stS"),
            st_v: g("stV"),
            tr_param: g("Ct3"),
        }
    }
    fn capture(system: &BTreeMap<String, String>) -> Self {
        let defaults = system_defaults();
        let g = |k: &str| {
            system.get(k).or_else(|| defaults.get(k)).cloned().unwrap_or_default()
        };
        MessageView {
            font_color: g("font_Color"),
            font_size: g("font_FontSize"),
            ruby_size: g("font_RubySize"),
            panel_mode: system.get("panel_Mode").cloned().unwrap_or_else(|| "0".into()),
            bg: g("stage_BGFilename"),
            bg_x: g("stage_BGX"),
            bg_y: g("stage_BGY"),
            panel_fade: "10".into(),
            bgm: g("bgm_Filename"),
            bgm_volume: g("bgm_Volume"),
            se1: g("se1_Filename"),
            se2: g("se2_Filename"),
            eff_count: g("eff_Count"),
            eff_speed: g("eff_Speed"),
            eff_param: g("eff_Param"),
            frame_param: g("frameParam"),
            bg_h: g("bgH"),
            bg_s: g("bgS"),
            bg_v: g("bgV"),
            st_h: g("stH"),
            st_s: g("stS"),
            st_v: g("stV"),
            tr_param: g("tr_Param"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacklogEntry {
    pub script: String,
    pub line: u32,
    pub no: u32,
    pub voice: String,
    pub speaker: String,
    pub body: String,
    #[serde(default)]
    pub view: MessageView,
}
impl BacklogEntry {
    pub fn to_raw(&self) -> eden_formats::save::BacklogEntryRaw {
        let f = |k: &str, v: &str| (k.to_string(), v.to_string());
        let v = &self.view;
        eden_formats::save::BacklogEntryRaw {
            fields: vec![
                f("LC", "0"), f("L0", & self.no.to_string()), f("L1", & self.voice),
                f("L2", & self.speaker), f("L3", & self.body), f("CS1", & self.script),
                f("CS2", & self.line.to_string()), f("CS3", & self.no.to_string()),
                f("CF1", & v.font_color), f("CF2", & v.font_size), f("CF3", & v
                .ruby_size), f("CP1", & v.panel_mode), f("CP2", ""), f("CT1", ""),
                f("CT2", & v.bg), f("CT3", & v.bg_x), f("CT4", & v.bg_y), f("CT5", & v
                .panel_fade), f("CT6", "0"), f("CT7", "0"), f("Frm", ""), f("FrP", & v
                .frame_param), f("CO0", ""), f("CO1", ""), f("CO2", ""), f("Cs0", ""),
                f("Cs1", ""), f("Cs2", ""), f("bgH", & v.bg_h), f("bgS", & v.bg_s),
                f("bgV", & v.bg_v), f("stH", & v.st_h), f("stS", & v.st_s), f("stV", & v
                .st_v), f("CB1", & v.bgm), f("CB1v", & v.bgm_volume), f("CE1", & v.se1),
                f("CE2", & v.se2), f("Ce1", ""), f("Ce2", ""), f("Ce3", & v.eff_count),
                f("Ce4", & v.eff_speed), f("Ce5", & v.eff_param), f("Ct1", "0"), f("Ct2",
                ""), f("Ct3", & v.tr_param),
            ],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveState {
    pub script: String,
    pub line: u32,
    pub message_no: Option<u32>,
    pub state: RunState,
    pub globals: BTreeMap<String, String>,
    pub locals: BTreeMap<String, String>,
    pub pragmas: BTreeSet<String>,
    pub system: BTreeMap<String, String>,
    pub backlog: Vec<BacklogEntry>,
}
pub struct Vm<H: Host> {
    pub(super) host: H,
    script_name: String,
    ir: ScriptIr,
    labels: HashMap<String, u32>,
    pc: u32,
    globals: BTreeMap<String, String>,
    locals: BTreeMap<String, String>,
    pragmas: BTreeSet<String>,
    pub(super) system: BTreeMap<String, String>,
    backlog: Vec<BacklogEntry>,
    state: RunState,
    last_message_no: Option<u32>,
    auto_delay: Option<u32>,
    auto_ticks: u32,
}
impl<H: Host> Vm<H> {
    pub fn new(host: H, script_name: impl Into<String>, ir: ScriptIr) -> Self {
        let labels = label_map(&ir);
        Vm {
            host,
            script_name: script_name.into(),
            ir,
            labels,
            pc: 1,
            globals: BTreeMap::new(),
            locals: BTreeMap::new(),
            pragmas: BTreeSet::new(),
            system: system_defaults(),
            backlog: Vec::new(),
            state: RunState::Running,
            last_message_no: None,
            auto_delay: None,
            auto_ticks: 0,
        }
    }
    pub fn host(&self) -> &H {
        &self.host
    }
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }
    pub fn script_name(&self) -> &str {
        &self.script_name
    }
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn current_message_no(&self) -> Option<u32> {
        self.last_message_no
    }
    pub fn state(&self) -> &RunState {
        &self.state
    }
    pub fn globals(&self) -> &BTreeMap<String, String> {
        &self.globals
    }
    pub fn seed_globals(&mut self, vars: &BTreeMap<String, String>) {
        for (k, v) in vars {
            self.globals.insert(k.clone(), v.clone());
        }
    }
    pub fn locals(&self) -> &BTreeMap<String, String> {
        &self.locals
    }
    pub fn pragma_enabled(&self, name: &str) -> bool {
        self.pragmas.contains(name)
    }
    pub fn backlog(&self) -> &[BacklogEntry] {
        &self.backlog
    }
    pub fn system_snapshot(&self) -> BTreeMap<String, String> {
        let mut m = self.system.clone();
        m.insert("script_Filename".into(), self.script_name.clone());
        m.insert("script_Pointer".into(), self.pc.to_string());
        if let Some(no) = self.last_message_no {
            m.insert("script_ID".into(), no.to_string());
        }
        m
    }
    pub fn restore_save(&mut self, data: &eden_formats::save::SaveData) {
        let mut line = self.pc;
        for (k, v) in &data.variables {
            match k.as_str() {
                "script_Pointer" => line = v.parse().unwrap_or(1),
                "script_ID" => self.last_message_no = v.parse().ok(),
                "script_Filename" => {}
                _ => {
                    self.system.insert(k.clone(), v.clone());
                }
            }
        }
        self.pc = line.clamp(1, self.ir.nodes.len() as u32);
        self.backlog = data
            .backlog
            .iter()
            .filter_map(|e| {
                Some(BacklogEntry {
                    script: e.get("CS1")?.to_string(),
                    line: e.get("CS2").and_then(|s| s.parse().ok()).unwrap_or(0),
                    no: e.get("L0").and_then(|s| s.parse().ok()).unwrap_or(0),
                    voice: e.get("L1").unwrap_or("").to_string(),
                    speaker: e.get("L2").unwrap_or("").to_string(),
                    body: e.get("L3").unwrap_or("").to_string(),
                    view: MessageView::from_raw(e),
                })
            })
            .collect();
        self.state = RunState::Running;
    }
    pub fn save_data(&self) -> eden_formats::save::SaveData {
        eden_formats::save::SaveData {
            variables: self.system_snapshot().into_iter().collect(),
            backlog: self.backlog.iter().map(|e| e.to_raw()).collect(),
        }
    }
    pub fn save(&self) -> SaveState {
        SaveState {
            script: self.script_name.clone(),
            line: self.pc,
            message_no: self.last_message_no,
            state: self.state.clone(),
            globals: self.globals.clone(),
            locals: self.locals.clone(),
            pragmas: self.pragmas.clone(),
            system: self.system.clone(),
            backlog: self.backlog.clone(),
        }
    }
    pub fn restore(
        &mut self,
        ir: ScriptIr,
        script_name: impl Into<String>,
        s: SaveState,
    ) {
        self.labels = label_map(&ir);
        self.ir = ir;
        self.script_name = script_name.into();
        self.pc = s.line.clamp(1, self.ir.nodes.len() as u32);
        self.globals = s.globals;
        self.locals = s.locals;
        self.pragmas = s.pragmas;
        self.system = s.system;
        self.backlog = s.backlog;
        self.state = s.state;
        self.last_message_no = s.message_no;
    }
    pub fn step(&mut self) -> Step {
        match self.state.clone() {
            RunState::Running => {}
            RunState::WaitFrames(f) => return Step::Wait { frames: f },
            RunState::AwaitInput => return Step::AwaitInput,
            RunState::AwaitAutoMessage { .. } => return Step::AwaitInput,
            RunState::WaitShake => return Step::WaitShake,
            RunState::WaitScroll => return Step::AwaitInput,
            RunState::Ended => return Step::Ended,
        }
        if self.pc as usize > self.ir.nodes.len() {
            self.state = RunState::Ended;
            return Step::Ended;
        }
        let kind = self.ir.nodes[self.pc as usize - 1].kind.clone();
        let step = self.dispatch(kind);
        match step {
            Step::Continue => self.pc += 1,
            Step::Jumped => {}
            other => return other,
        }
        if self.pc as usize > self.ir.nodes.len() {
            self.state = RunState::Ended;
            return Step::Ended;
        }
        if step == Step::Jumped {
            return Step::Continue;
        }
        step
    }
    pub fn pump(&mut self, frames: u32) -> RunState {
        if let RunState::WaitFrames(n) = self.state {
            let n = n.saturating_sub(frames);
            if n == 0 {
                self.pc += 1;
                if self.pc as usize > self.ir.nodes.len() {
                    self.state = RunState::Ended;
                } else {
                    self.state = RunState::Running;
                }
            } else {
                self.state = RunState::WaitFrames(n);
            }
        }
        self.state.clone()
    }
    pub fn advance_input(&mut self) {
        if matches!(
            self.state, RunState::AwaitInput | RunState::AwaitAutoMessage { .. }
        ) {
            self.pc += 1;
            self.auto_ticks = 0;
            if self.pc as usize > self.ir.nodes.len() {
                self.state = RunState::Ended;
            } else {
                self.state = RunState::Running;
            }
        }
    }
    pub fn advance_scroll(&mut self) {
        if self.state == RunState::WaitScroll {
            self.pc += 1;
            if self.pc as usize > self.ir.nodes.len() {
                self.state = RunState::Ended;
            } else {
                self.state = RunState::Running;
            }
        }
    }
    pub fn advance_shake(&mut self) {
        if self.state == RunState::WaitShake {
            self.pc += 1;
            if self.pc as usize > self.ir.nodes.len() {
                self.state = RunState::Ended;
            } else {
                self.state = RunState::Running;
            }
        }
    }
    pub fn set_auto(&mut self, delay_ticks: Option<u32>) {
        self.auto_delay = delay_ticks;
        self.auto_ticks = 0;
    }
    pub fn tick(&mut self) -> RunState {
        if matches!(self.state, RunState::WaitFrames(_)) {
            self.pump(1);
        }
        if self.state == RunState::AwaitInput {
            if let Some(delay) = self.auto_delay {
                self.auto_ticks = self.auto_ticks.saturating_add(1);
                if self.auto_ticks >= delay {
                    self.advance_input();
                }
            }
        }
        if let RunState::AwaitAutoMessage { wait_voice } = self.state {
            if self.host.message_complete()
                && (!wait_voice || !self.host.voice_playing())
            {
                self.advance_input();
            }
        }
        self.state.clone()
    }
    fn dispatch(&mut self, kind: NodeKind) -> Step {
        use NodeKind::*;
        match kind {
            NoOp(_) | Label(_) => Step::Continue,
            Message(_) => handlers::text::exec(self),
            Stage(_)
            | Transition(_)
            | Vscroll(_)
            | Hscroll(_)
            | Scroll(_)
            | EndScroll(_)
            | ShakeScreen(_)
            | Effect(_) => handlers::scene::exec(self),
            PlayBgm(_) | PlaySe(_) | PlayVoice(_) | Movie(_) => handlers::av::exec(self),
            SetGlobal(_) | Set(_) | Pragma(_) | Panel(_) | Wait(_) | Raw(_) => {
                handlers::system::exec(self)
            }
            Goto(_) | If(_) | Chain(_) | End => handlers::flow::exec(self),
        }
    }
    pub(super) fn jump_to_label(&mut self, name: &str) -> bool {
        if let Some(&line) = self.labels.get(name) {
            self.pc = line;
            true
        } else {
            log::warn!("label miss: {name}（{}:{})", self.script_name, self.pc);
            false
        }
    }
    fn resolve_operand(&self, t: &str) -> String {
        self.locals
            .get(t)
            .or_else(|| self.globals.get(t))
            .cloned()
            .unwrap_or_else(|| t.to_string())
    }
    pub(super) fn eval_if(&self, i: &If) -> bool {
        let l = self.resolve_operand(&i.var);
        let r_raw = i.value.to_string();
        let r = self.resolve_operand(&r_raw);
        let (li, ri) = (atoi_def0(&l), atoi_def0(&r));
        use eden_formats::script::CmpOp::*;
        match i.op {
            Eq => li == ri || l == r,
            Ne => li != ri,
            Le => li <= ri,
            Ge => li >= ri,
            Lt => li < ri,
            Gt => li > ri,
        }
    }
    pub(super) fn set_var(
        &mut self,
        global: bool,
        var: &str,
        value: &eden_formats::script::AssignValue,
    ) {
        use eden_formats::script::AssignValue;
        let v = match value {
            AssignValue::Simple(a) => a.to_string(),
            AssignValue::Binary { lhs, op, rhs } => {
                let lt = lhs.to_string();
                let rt = rhs.to_string();
                let (l, r) = (
                    atoi_def0(&self.resolve_operand(&lt)),
                    atoi_def0(&self.resolve_operand(&rt)),
                );
                use eden_formats::script::BinOp::*;
                let out = match op {
                    Or => l | r,
                    And => l & r,
                    Add => l.wrapping_add(r),
                    Sub => l.wrapping_sub(r),
                    Mul => l.wrapping_mul(r),
                    Div | Mod => {
                        if r == 0 {
                            log::warn!(
                                "set 表达式除零: {var}（{}:{}）", self.script_name,
                                self.pc
                            );
                            0
                        } else if *op == Div {
                            l / r
                        } else {
                            l % r
                        }
                    }
                };
                out.to_string()
            }
        };
        if global {
            self.globals.insert(var.to_string(), v);
        } else {
            self.locals.insert(var.to_string(), v);
        }
    }
}
pub fn atoi_def0(s: &str) -> i64 {
    s.trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '-' && c != '+')
        .next()
        .and_then(|t| t.parse::<i64>().ok())
        .unwrap_or(0)
}
fn system_defaults() -> BTreeMap<String, String> {
    let kv: &[(&str, &str)] = &[
        ("angBG", "0000000000000000"),
        ("angFG", "0000000000000000"),
        ("bgH", "0"),
        ("bgS", "0"),
        ("bgV", "0"),
        ("bgm_Volume", "100"),
        ("eff_Count", "-1"),
        ("eff_Param", "-1"),
        ("eff_Speed", "-1"),
        ("font_Color", "16777215"),
        ("font_FontSize", "22"),
        ("font_RubySize", "10"),
        ("frameParam", "-1"),
        ("rotBG_dest.x", "0"),
        ("rotBG_dest.y", "0"),
        ("rotBG_source.x", "0"),
        ("rotBG_source.y", "0"),
        ("rotFG_dest.x", "0"),
        ("rotFG_dest.y", "0"),
        ("rotFG_source.x", "0"),
        ("rotFG_source.y", "0"),
        ("stH", "0"),
        ("stS", "0"),
        ("stV", "0"),
    ];
    kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}
fn label_map(ir: &ScriptIr) -> HashMap<String, u32> {
    ir.nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Label(l) => Some((l.name.clone(), n.line)),
            _ => None,
        })
        .collect()
}
