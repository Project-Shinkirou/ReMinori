use eden_formats::script::{
    Effect, Message, Movie, Panel, PlayBgm, PlaySe, Raw, Stage, Transition,
};
pub trait Host {
    fn message(&mut self, msg: &Message, is_read: bool);
    fn message_is_read(&self, _message_no: u32) -> bool {
        false
    }
    fn message_complete(&self) -> bool {
        true
    }
    fn voice_playing(&self) -> bool {
        false
    }
    fn stage(&mut self, s: &Stage);
    fn transition(&mut self, t: &Transition);
    fn scroll_vh(&mut self, vertical: bool, a: i64, b: i64);
    fn scroll(&mut self, x: i64, y: i64, n: i64);
    fn end_scroll(&mut self, flag: bool) -> bool;
    fn shake_screen(&mut self, mode: &str, a: i64, b: i64);
    fn effect(&mut self, e: &Effect);
    fn play_bgm(&mut self, p: &PlayBgm);
    fn play_se(&mut self, p: &PlaySe);
    fn play_voice(&mut self, name: &str, rest: &[eden_formats::script::Arg]);
    fn movie(&mut self, m: &Movie);
    fn panel(&mut self, p: &Panel) -> bool;
    fn pragma(&mut self, name: &str);
    fn end(&mut self);
    fn load_script(&mut self, name: &str) -> Option<eden_formats::ScriptIr>;
    fn unknown(&mut self, _raw: &Raw) {}
}
