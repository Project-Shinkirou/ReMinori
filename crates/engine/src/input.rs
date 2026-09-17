#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKey {
    Enter,
    Space,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    Control,
    F5,
    F9,
    Character(String),
    Other,
}
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerMoved { x: f32, y: f32 },
    PointerLeft,
    PointerButton { button: PointerButton, state: ButtonState, click_count: u8 },
    WheelSteps { x: f32, y: f32 },
    Key { key: InputKey, state: ButtonState, repeat: bool },
}
