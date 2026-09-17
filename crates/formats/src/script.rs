use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
pub const KNOWN_TAGS: &[&str] = &[
    "setGlobal",
    "set",
    "if",
    "label",
    "chain",
    "end",
    "stage",
    "transition",
    "wait",
    "message",
    "playBGM",
    "playSE",
    "playSE2",
    "panel",
    "pragma",
    "goto",
    "endScroll",
    "vscroll",
    "hscroll",
    "scroll",
    "playVoice",
    "shakeScreen",
    "effect",
    "movie",
];
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptIr {
    pub nodes: Vec<Node>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub line: u32,
    pub kind: NodeKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    NoOp(NoOp),
    Message(Message),
    SetGlobal(Assign),
    Set(Assign),
    If(If),
    Label(Label),
    Chain(Chain),
    End,
    Stage(Stage),
    Transition(Transition),
    Wait(Wait),
    PlayBgm(PlayBgm),
    PlaySe(PlaySe),
    Panel(Panel),
    Pragma(Pragma),
    Goto(Goto),
    EndScroll(EndScroll),
    Vscroll(Scroll2),
    Hscroll(Scroll2),
    Scroll(Scroll3),
    PlayVoice(PlayVoice),
    ShakeScreen(ShakeScreen),
    Effect(Effect),
    Movie(Movie),
    Raw(Raw),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoOp {
    pub kind: NoOpKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NoOpKind {
    Comment(String),
    Blank,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Arg {
    Star,
    Int(i64),
    Word(String),
}
impl Arg {
    fn parse(s: &str) -> Arg {
        if s == "*" {
            Arg::Star
        } else if let Ok(v) = s.parse::<i64>() {
            if v.to_string() == s { Arg::Int(v) } else { Arg::Word(s.to_string()) }
        } else {
            Arg::Word(s.to_string())
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Arg::Int(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_flag(&self) -> Option<bool> {
        let c = match self {
            Arg::Word(s) => s.chars().next()?,
            _ => return None,
        };
        match c {
            '1'..='9' | 't' | 'T' => Some(true),
            '0' | 'f' | 'F' => Some(false),
            _ => None,
        }
    }
    pub fn script_int(&self, default: i64) -> i64 {
        match self {
            Arg::Int(v) => *v,
            Arg::Word(s) => scan_int_prefix(s).unwrap_or(default),
            Arg::Star => default,
        }
    }
}
fn scan_int_prefix(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = usize::from(bytes.first().is_some_and(|b| *b == b'+' || *b == b'-'));
    let digits = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    (i > digits).then(|| s[..i].parse().ok()).flatten()
}
impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Star => write!(f, "*"),
            Arg::Int(v) => write!(f, "{v}"),
            Arg::Word(s) => write!(f, "{s}"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub no: u32,
    pub voice: Option<VoiceRef>,
    pub speaker: Option<Speaker>,
    pub body: Vec<BodyToken>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceRef {
    pub name: String,
    pub coords: Vec<i32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    pub sep: Option<SpeakerSep>,
    pub text: String,
}
impl Speaker {
    pub fn token(&self) -> String {
        match self.sep {
            Some(s) => format!("{}{}", s.ch(), self.text),
            None => self.text.clone(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeakerSep {
    At,
    Hash,
}
impl SpeakerSep {
    pub fn ch(self) -> char {
        match self {
            SpeakerSep::At => '@',
            SpeakerSep::Hash => '#',
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BodyToken {
    Text(String),
    Ctrl(String),
}
impl Message {
    pub fn body_text(&self) -> String {
        self.body
            .iter()
            .map(|t| match t {
                BodyToken::Text(s) | BodyToken::Ctrl(s) => s.as_str(),
            })
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assign {
    pub var: String,
    pub value: AssignValue,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssignValue {
    Simple(Arg),
    Binary { lhs: Arg, op: BinOp, rhs: Arg },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Or,
    And,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}
impl BinOp {
    fn parse(s: &str) -> Option<BinOp> {
        Some(
            match s {
                "|" => BinOp::Or,
                "&" => BinOp::And,
                "+" => BinOp::Add,
                "-" => BinOp::Sub,
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                "%" => BinOp::Mod,
                _ => return None,
            },
        )
    }
}
impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BinOp::Or => "|",
            BinOp::And => "&",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
        };
        write!(f, "{s}")
    }
}
impl fmt::Display for AssignValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssignValue::Simple(a) => write!(f, "{a}"),
            AssignValue::Binary { lhs, op, rhs } => write!(f, "{lhs} {op} {rhs}"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct If {
    pub var: String,
    pub op: CmpOp,
    pub value: Arg,
    pub target: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
}
impl CmpOp {
    fn parse(s: &str) -> Option<CmpOp> {
        Some(
            match s {
                "==" => CmpOp::Eq,
                "!=" => CmpOp::Ne,
                "<=" => CmpOp::Le,
                ">=" => CmpOp::Ge,
                "<" => CmpOp::Lt,
                ">" => CmpOp::Gt,
                _ => return None,
            },
        )
    }
}
impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
        };
        write!(f, "{s}")
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    pub script: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub layer: Arg,
    pub file: Arg,
    pub x: Arg,
    pub y: Arg,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: Arg,
    pub mask: Arg,
    pub frames: Arg,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wait {
    pub frames: Arg,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayBgm {
    pub file: Arg,
    pub rest: Vec<Arg>,
}
impl PlayBgm {
    pub fn fade_in(&self) -> i64 {
        self.rest.first().map_or(1, |a| a.script_int(2))
    }
    pub fn fade_out(&self) -> i64 {
        self.rest.get(1).map_or(1, |a| a.script_int(2))
    }
    pub fn volume(&self) -> i64 {
        self.rest.get(2).map_or(100, |a| a.script_int(100))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaySe {
    pub se2: bool,
    pub file: Arg,
    pub volume: Option<i64>,
    pub rest: Vec<Arg>,
}
impl PlaySe {
    pub fn looping(&self) -> bool {
        matches!(
            self.rest.first(), Some(Arg::Word(s)) if s.as_bytes().first() == Some(& b't')
        )
    }
    pub fn fade_in(&self) -> i64 {
        self.rest.get(1).map_or(2, |a| a.script_int(2))
    }
    pub fn fade_out(&self) -> i64 {
        self.rest.get(2).map_or(2, |a| a.script_int(2))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub args: Vec<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pragma {
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Raw {
    pub tag: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goto {
    pub target: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndScroll {
    pub flag: Arg,
    pub rest: Vec<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scroll2 {
    pub a: Arg,
    pub b: Option<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scroll3 {
    pub x: Arg,
    pub y: Option<Arg>,
    pub n: Option<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayVoice {
    pub name: Arg,
    pub rest: Vec<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShakeScreen {
    pub mode: Arg,
    pub a: Arg,
    pub b: Arg,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub name: Arg,
    pub name2: Option<Arg>,
    pub ints: Vec<Arg>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movie {
    pub id: Arg,
    pub file: Arg,
    pub w: Option<Arg>,
    pub h: Option<Arg>,
    pub skip: Option<Arg>,
    pub x: Option<Arg>,
    pub y: Option<Arg>,
}
impl ScriptIr {
    pub fn parse_bytes(bytes: &[u8]) -> ScriptIr {
        Self::parse(&crate::encoding::cp932_to_string(bytes))
    }
    pub fn parse(text: &str) -> ScriptIr {
        let nodes = text
            .lines()
            .enumerate()
            .map(|(i, line)| Node {
                line: i as u32 + 1,
                kind: parse_line(line),
            })
            .collect();
        ScriptIr { nodes }
    }
    pub fn serialize(&self) -> String {
        let mut s = String::new();
        for n in &self.nodes {
            s.push_str(&n.kind.to_line());
            s.push_str("\r\n");
        }
        s
    }
    pub fn stats(&self) -> ScriptStats {
        let mut st = ScriptStats::default();
        for n in &self.nodes {
            st.lines += 1;
            match &n.kind {
                NodeKind::NoOp(_) => {}
                NodeKind::Raw(r) => {
                    st.command_lines += 1;
                    st.raw += 1;
                    *st.raw_by_tag.entry(r.tag.clone()).or_insert(0) += 1;
                    if KNOWN_TAGS.contains(&r.tag.as_str()) {
                        st.known_tag_lines += 1;
                    }
                }
                kind => {
                    st.command_lines += 1;
                    st.structured += 1;
                    if KNOWN_TAGS.contains(&kind.tag()) {
                        st.known_tag_lines += 1;
                        st.known_tag_structured += 1;
                    }
                }
            }
        }
        st
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptStats {
    pub lines: usize,
    pub command_lines: usize,
    pub structured: usize,
    pub raw: usize,
    pub known_tag_lines: usize,
    pub known_tag_structured: usize,
    pub raw_by_tag: BTreeMap<String, usize>,
}
impl ScriptStats {
    pub fn known_structured_rate(&self) -> Option<f64> {
        if self.known_tag_lines == 0 {
            None
        } else {
            Some(self.known_tag_structured as f64 / self.known_tag_lines as f64)
        }
    }
    pub fn merge(&mut self, other: &ScriptStats) {
        self.lines += other.lines;
        self.command_lines += other.command_lines;
        self.structured += other.structured;
        self.raw += other.raw;
        self.known_tag_lines += other.known_tag_lines;
        self.known_tag_structured += other.known_tag_structured;
        for (k, v) in &other.raw_by_tag {
            *self.raw_by_tag.entry(k.clone()).or_insert(0) += v;
        }
    }
}
impl NodeKind {
    pub fn tag(&self) -> &str {
        match self {
            NodeKind::NoOp(_) => "",
            NodeKind::Message(_) => "message",
            NodeKind::SetGlobal(_) => "setGlobal",
            NodeKind::Set(_) => "set",
            NodeKind::If(_) => "if",
            NodeKind::Label(_) => "label",
            NodeKind::Chain(_) => "chain",
            NodeKind::End => "end",
            NodeKind::Stage(_) => "stage",
            NodeKind::Transition(_) => "transition",
            NodeKind::Wait(_) => "wait",
            NodeKind::PlayBgm(_) => "playBGM",
            NodeKind::PlaySe(se) => if se.se2 { "playSE2" } else { "playSE" }
            NodeKind::Panel(_) => "panel",
            NodeKind::Pragma(_) => "pragma",
            NodeKind::Goto(_) => "goto",
            NodeKind::EndScroll(_) => "endScroll",
            NodeKind::Vscroll(_) => "vscroll",
            NodeKind::Hscroll(_) => "hscroll",
            NodeKind::Scroll(_) => "scroll",
            NodeKind::PlayVoice(_) => "playVoice",
            NodeKind::ShakeScreen(_) => "shakeScreen",
            NodeKind::Effect(_) => "effect",
            NodeKind::Movie(_) => "movie",
            NodeKind::Raw(r) => r.tag.as_str(),
        }
    }
    pub fn to_line(&self) -> String {
        fn join(tag: &str, parts: &[String]) -> String {
            let mut s = String::from(tag);
            for p in parts {
                s.push(' ');
                s.push_str(p);
            }
            s
        }
        match self {
            NodeKind::NoOp(n) => {
                match &n.kind {
                    NoOpKind::Comment(t) => format!(";{t}"),
                    NoOpKind::Blank => String::new(),
                }
            }
            NodeKind::Message(m) => {
                let mut s = format!(".message {}", m.no);
                s.push(' ');
                if let Some(v) = &m.voice {
                    s.push_str(&v.name);
                    if !v.coords.is_empty() {
                        let cs: Vec<String> = v
                            .coords
                            .iter()
                            .map(|c| c.to_string())
                            .collect();
                        s.push('[');
                        s.push_str(&cs.join(","));
                        s.push(']');
                    }
                }
                s.push(' ');
                if let Some(sp) = &m.speaker {
                    s.push_str(&sp.token());
                }
                s.push(' ');
                s.push_str(&m.body_text());
                s
            }
            NodeKind::SetGlobal(a) => {
                join(".setGlobal", &[a.var.clone(), "=".into(), a.value.to_string()])
            }
            NodeKind::Set(a) => {
                join(".set", &[a.var.clone(), "=".into(), a.value.to_string()])
            }
            NodeKind::If(i) => {
                join(
                    ".if",
                    &[
                        i.var.clone(),
                        i.op.to_string(),
                        i.value.to_string(),
                        i.target.clone(),
                    ],
                )
            }
            NodeKind::Label(l) => join(".label", std::slice::from_ref(&l.name)),
            NodeKind::Chain(c) => join(".chain", std::slice::from_ref(&c.script)),
            NodeKind::End => ".end".into(),
            NodeKind::Stage(t) => {
                join(
                    ".stage",
                    &[
                        t.layer.to_string(),
                        t.file.to_string(),
                        t.x.to_string(),
                        t.y.to_string(),
                    ],
                )
            }
            NodeKind::Transition(t) => {
                join(
                    ".transition",
                    &[t.kind.to_string(), t.mask.to_string(), t.frames.to_string()],
                )
            }
            NodeKind::Wait(w) => join(".wait", &[w.frames.to_string()]),
            NodeKind::PlayBgm(p) => {
                let mut parts = vec![p.file.to_string()];
                parts.extend(p.rest.iter().map(|a| a.to_string()));
                join(".playBGM", &parts)
            }
            NodeKind::PlaySe(p) => {
                let mut parts = vec![p.file.to_string()];
                parts.extend(p.rest.iter().map(|a| a.to_string()));
                join(if p.se2 { ".playSE2" } else { ".playSE" }, &parts)
            }
            NodeKind::Panel(p) => {
                let parts: Vec<String> = p.args.iter().map(|a| a.to_string()).collect();
                join(".panel", &parts)
            }
            NodeKind::Pragma(p) => join(".pragma", std::slice::from_ref(&p.name)),
            NodeKind::Goto(g) => join(".goto", std::slice::from_ref(&g.target)),
            NodeKind::EndScroll(e) => {
                let mut parts = vec![e.flag.to_string()];
                parts.extend(e.rest.iter().map(|a| a.to_string()));
                join(".endScroll", &parts)
            }
            NodeKind::Vscroll(s) => {
                let mut parts = vec![s.a.to_string()];
                parts.extend(s.b.iter().map(|a| a.to_string()));
                join(".vscroll", &parts)
            }
            NodeKind::Hscroll(s) => {
                let mut parts = vec![s.a.to_string()];
                parts.extend(s.b.iter().map(|a| a.to_string()));
                join(".hscroll", &parts)
            }
            NodeKind::Scroll(s) => {
                let mut parts = vec![s.x.to_string()];
                parts.extend(s.y.iter().map(|a| a.to_string()));
                parts.extend(s.n.iter().map(|a| a.to_string()));
                join(".scroll", &parts)
            }
            NodeKind::PlayVoice(p) => {
                let mut parts = vec![p.name.to_string()];
                parts.extend(p.rest.iter().map(|a| a.to_string()));
                join(".playVoice", &parts)
            }
            NodeKind::ShakeScreen(s) => {
                join(
                    ".shakeScreen",
                    &[s.mode.to_string(), s.a.to_string(), s.b.to_string()],
                )
            }
            NodeKind::Effect(e) => {
                let mut parts = vec![e.name.to_string()];
                parts.extend(e.name2.iter().map(|a| a.to_string()));
                parts.extend(e.ints.iter().map(|a| a.to_string()));
                join(".effect", &parts)
            }
            NodeKind::Movie(m) => {
                let mut parts = vec![m.id.to_string(), m.file.to_string()];
                for a in [&m.w, &m.h, &m.skip, &m.x, &m.y].into_iter().flatten() {
                    parts.push(a.to_string());
                }
                join(".movie", &parts)
            }
            NodeKind::Raw(r) => r.text.clone(),
        }
    }
}
fn parse_line(line: &str) -> NodeKind {
    if line.is_empty() {
        return NodeKind::NoOp(NoOp { kind: NoOpKind::Blank });
    }
    if let Some(rest) = line.strip_prefix(';') {
        return NodeKind::NoOp(NoOp {
            kind: NoOpKind::Comment(rest.to_string()),
        });
    }
    if let Some(rest) = line.strip_prefix('.') {
        let (tag, args_str) = match rest.find(' ') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        match parse_command(tag, args_str) {
            Some(kind) => kind,
            None => {
                NodeKind::Raw(Raw {
                    tag: tag.to_string(),
                    text: line.to_string(),
                })
            }
        }
    } else {
        NodeKind::Raw(Raw {
            tag: String::new(),
            text: line.to_string(),
        })
    }
}
fn parse_command(tag: &str, args_str: &str) -> Option<NodeKind> {
    if tag == "message" {
        return parse_message(args_str).map(NodeKind::Message);
    }
    let args: Vec<Arg> = if args_str.is_empty() {
        Vec::new()
    } else {
        args_str.split(' ').map(Arg::parse).collect()
    };
    let word = |a: &Arg| match a {
        Arg::Word(w) => w.clone(),
        other => other.to_string(),
    };
    match tag {
        "setGlobal" | "set" => {
            let value = if args.len() == 3 && args[1] == Arg::Word("=".into()) {
                AssignValue::Simple(args[2].clone())
            } else if args.len() == 5 && args[1] == Arg::Word("=".into())
                && BinOp::parse(&args[3].to_string()).is_some()
            {
                AssignValue::Binary {
                    lhs: args[2].clone(),
                    op: BinOp::parse(&args[3].to_string()).unwrap(),
                    rhs: args[4].clone(),
                }
            } else {
                return None;
            };
            let a = Assign {
                var: word(&args[0]),
                value,
            };
            Some(
                if tag == "setGlobal" {
                    NodeKind::SetGlobal(a)
                } else {
                    NodeKind::Set(a)
                },
            )
        }
        "if" => {
            if args.len() == 4 {
                Some(
                    NodeKind::If(If {
                        var: word(&args[0]),
                        op: CmpOp::parse(&args[1].to_string())?,
                        value: args[2].clone(),
                        target: word(&args[3]),
                    }),
                )
            } else {
                None
            }
        }
        "label" => {
            (args.len() == 1)
                .then(|| { NodeKind::Label(Label { name: word(&args[0]) }) })
        }
        "chain" => {
            (args.len() == 1)
                .then(|| { NodeKind::Chain(Chain { script: word(&args[0]) }) })
        }
        "end" => (args.is_empty()).then_some(NodeKind::End),
        "stage" => {
            (args.len() == 4)
                .then(|| {
                    NodeKind::Stage(Stage {
                        layer: args[0].clone(),
                        file: args[1].clone(),
                        x: args[2].clone(),
                        y: args[3].clone(),
                    })
                })
        }
        "transition" => {
            (args.len() == 3)
                .then(|| {
                    NodeKind::Transition(Transition {
                        kind: args[0].clone(),
                        mask: args[1].clone(),
                        frames: args[2].clone(),
                    })
                })
        }
        "wait" => {
            (args.len() == 1)
                .then(|| { NodeKind::Wait(Wait { frames: args[0].clone() }) })
        }
        "playBGM" => {
            (!args.is_empty())
                .then(|| {
                    let mut it = args.into_iter();
                    let file = it.next().unwrap();
                    NodeKind::PlayBgm(PlayBgm {
                        file,
                        rest: it.collect(),
                    })
                })
        }
        "playSE" | "playSE2" => {
            parse_playse(tag == "playSE2", args).map(NodeKind::PlaySe)
        }
        "panel" => (!args.is_empty()).then_some(NodeKind::Panel(Panel { args })),
        "pragma" => {
            (args.len() == 1)
                .then(|| { NodeKind::Pragma(Pragma { name: word(&args[0]) }) })
        }
        "goto" => {
            (args.len() == 1)
                .then(|| { NodeKind::Goto(Goto { target: word(&args[0]) }) })
        }
        "endScroll" => {
            (!args.is_empty())
                .then(|| {
                    let mut it = args.into_iter();
                    let flag = it.next().unwrap();
                    NodeKind::EndScroll(EndScroll {
                        flag,
                        rest: it.collect(),
                    })
                })
        }
        "vscroll" | "hscroll" => {
            ((1..=2).contains(&args.len()))
                .then(|| {
                    let s = Scroll2 {
                        a: args[0].clone(),
                        b: args.get(1).cloned(),
                    };
                    if tag == "vscroll" {
                        NodeKind::Vscroll(s)
                    } else {
                        NodeKind::Hscroll(s)
                    }
                })
        }
        "scroll" => {
            ((1..=3).contains(&args.len()))
                .then(|| {
                    NodeKind::Scroll(Scroll3 {
                        x: args[0].clone(),
                        y: args.get(1).cloned(),
                        n: args.get(2).cloned(),
                    })
                })
        }
        "playVoice" => {
            (!args.is_empty())
                .then(|| {
                    let mut it = args.into_iter();
                    let name = it.next().unwrap();
                    NodeKind::PlayVoice(PlayVoice {
                        name,
                        rest: it.collect(),
                    })
                })
        }
        "shakeScreen" => {
            (args.len() == 3)
                .then(|| {
                    NodeKind::ShakeScreen(ShakeScreen {
                        mode: args[0].clone(),
                        a: args[1].clone(),
                        b: args[2].clone(),
                    })
                })
        }
        "effect" => {
            ((1..=5).contains(&args.len()))
                .then(|| {
                    let name2 = args.get(1).cloned();
                    let ints = if args.len() > 2 {
                        args[2..].to_vec()
                    } else {
                        Vec::new()
                    };
                    NodeKind::Effect(Effect {
                        name: args[0].clone(),
                        name2,
                        ints,
                    })
                })
        }
        "movie" => {
            ((4..=7).contains(&args.len()))
                .then(|| {
                    NodeKind::Movie(Movie {
                        id: args[0].clone(),
                        file: args[1].clone(),
                        w: args.get(2).cloned(),
                        h: args.get(3).cloned(),
                        skip: args.get(4).cloned(),
                        x: args.get(5).cloned(),
                        y: args.get(6).cloned(),
                    })
                })
        }
        _ => None,
    }
}
fn parse_message(args_str: &str) -> Option<Message> {
    let digits: usize = args_str.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let no: u32 = args_str[..digits].parse().ok()?;
    if args_str.as_bytes().get(digits) != Some(&b' ') {
        return None;
    }
    let rest = &args_str[digits + 1..];
    let mut fields = rest.splitn(3, ' ');
    let voice_tok = fields.next().unwrap_or("");
    let speaker_tok = fields.next().unwrap_or("");
    let body_str = fields.next().unwrap_or("");
    let voice = (!voice_tok.is_empty()).then(|| parse_voice(voice_tok));
    let speaker = parse_speaker(speaker_tok);
    Some(Message {
        no,
        voice,
        speaker,
        body: split_body(body_str),
    })
}
fn parse_speaker(tok: &str) -> Option<Speaker> {
    match tok.chars().next() {
        None => None,
        Some('@') => {
            Some(Speaker {
                sep: Some(SpeakerSep::At),
                text: tok[1..].to_string(),
            })
        }
        Some('#') => {
            Some(Speaker {
                sep: Some(SpeakerSep::Hash),
                text: tok[1..].to_string(),
            })
        }
        Some(_) => {
            Some(Speaker {
                sep: None,
                text: tok.to_string(),
            })
        }
    }
}
fn parse_voice(tok: &str) -> VoiceRef {
    if let Some(i) = tok.find('[') {
        if let Some(c) = tok[i + 1..].strip_suffix(']') {
            let coords: Option<Vec<i32>> = c
                .split(',')
                .map(|p| p.parse::<i32>().ok())
                .collect();
            if let Some(coords) = coords {
                return VoiceRef {
                    name: tok[..i].to_string(),
                    coords,
                };
            }
        }
    }
    VoiceRef {
        name: tok.to_string(),
        coords: Vec::new(),
    }
}
fn split_body(s: &str) -> Vec<BodyToken> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some(&(_, next)) = chars.peek() {
                if !text.is_empty() {
                    out.push(BodyToken::Text(std::mem::take(&mut text)));
                }
                let n = next.len_utf8();
                out.push(BodyToken::Ctrl(s[i..i + 1 + n].to_string()));
                chars.next();
                continue;
            }
        }
        text.push(c);
    }
    if !text.is_empty() {
        out.push(BodyToken::Text(text));
    }
    out
}
fn parse_playse(se2: bool, args: Vec<Arg>) -> Option<PlaySe> {
    if args.is_empty() {
        return None;
    }
    let mut it = args.into_iter();
    let file = it.next().unwrap();
    Some(PlaySe {
        se2,
        file,
        volume: None,
        rest: it.collect(),
    })
}

