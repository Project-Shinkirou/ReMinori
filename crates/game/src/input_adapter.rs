use std::time::{Duration, Instant};
use eden_engine::{ButtonState, InputEvent, InputKey, PointerButton, Viewport};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key, NamedKey};
use winit::window::Window;
const PIXELS_PER_WHEEL_STEP: f32 = 100.0;
#[derive(Debug, Clone, Copy)]
struct LastPress {
    button: PointerButton,
    at: Instant,
    x: f32,
    y: f32,
}
#[derive(Debug, Default)]
pub struct InputAdapter {
    cursor: Option<(f32, f32)>,
    cursor_physical: Option<(f32, f32)>,
    last_press: Option<LastPress>,
    wheel_x: f32,
    wheel_y: f32,
}
impl InputAdapter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pointer_moved(
        &mut self,
        window: &Window,
        position: PhysicalPosition<f64>,
    ) -> InputEvent {
        let p = self.logical_position(window, position);
        self.cursor = Some(p);
        self.cursor_physical = Some((position.x as f32, position.y as f32));
        InputEvent::PointerMoved {
            x: p.0,
            y: p.1,
        }
    }
    pub fn pointer_left(&mut self) -> InputEvent {
        self.cursor = None;
        self.cursor_physical = None;
        InputEvent::PointerLeft
    }
    pub fn mouse_button(
        &mut self,
        button: MouseButton,
        state: ElementState,
    ) -> InputEvent {
        let button = pointer_button(button);
        let click_count = if state == ElementState::Pressed {
            let now = Instant::now();
            let (double_click_time, double_click_x, double_click_y) = double_click_limits();
            let count = self
                .last_press
                .filter(|last| {
                    last.button == button
                        && now.saturating_duration_since(last.at) <= double_click_time
                        && self
                            .cursor_physical
                            .is_some_and(|(x, y)| {
                                (x - last.x).abs() <= double_click_x
                                    && (y - last.y).abs() <= double_click_y
                            })
                })
                .map_or(1, |_| 2);
            let (x, y) = self.cursor_physical.unwrap_or((0.0, 0.0));
            self.last_press = (count == 1)
                .then_some(LastPress { button, at: now, x, y });
            count
        } else {
            1
        };
        InputEvent::PointerButton {
            button,
            state: button_state(state),
            click_count,
        }
    }
    pub fn wheel(&mut self, delta: MouseScrollDelta) -> Option<InputEvent> {
        match delta {
            MouseScrollDelta::LineDelta(x, y) => Some(InputEvent::WheelSteps { x, y }),
            MouseScrollDelta::PixelDelta(position) => {
                self.wheel_x += position.x as f32 / PIXELS_PER_WHEEL_STEP;
                self.wheel_y += position.y as f32 / PIXELS_PER_WHEEL_STEP;
                let x = whole_steps(&mut self.wheel_x);
                let y = whole_steps(&mut self.wheel_y);
                (x != 0.0 || y != 0.0).then_some(InputEvent::WheelSteps { x, y })
            }
        }
    }
    pub fn keyboard(&self, event: &KeyEvent) -> Option<InputEvent> {
        let key = input_key(&event.logical_key)?;
        Some(InputEvent::Key {
            key,
            state: button_state(event.state),
            repeat: event.repeat,
        })
    }
    fn logical_position(
        &self,
        window: &Window,
        position: PhysicalPosition<f64>,
    ) -> (f32, f32) {
        let size = window.inner_size();
        Viewport::new(size.width, size.height)
            .to_logical(position.x as f32, position.y as f32)
    }
}
#[cfg(windows)]
fn double_click_limits() -> (Duration, f32, f32) {
    eden_engine::platform::windows::double_click_limits()
}
#[cfg(not(windows))]
fn double_click_limits() -> (Duration, f32, f32) {
    (Duration::from_millis(500), 2.0, 2.0)
}
fn whole_steps(value: &mut f32) -> f32 {
    let steps = if *value >= 0.0 { value.floor() } else { value.ceil() };
    *value -= steps;
    steps
}
fn pointer_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Left,
        MouseButton::Right => PointerButton::Right,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Back,
        MouseButton::Forward => PointerButton::Forward,
        MouseButton::Other(value) => PointerButton::Other(value),
    }
}
fn input_key(key: &Key) -> Option<InputKey> {
    Some(
        match key {
            Key::Named(named) => {
                match named {
                    NamedKey::Enter => InputKey::Enter,
                    NamedKey::Space => InputKey::Space,
                    NamedKey::Escape => InputKey::Escape,
                    NamedKey::ArrowUp => InputKey::ArrowUp,
                    NamedKey::ArrowDown => InputKey::ArrowDown,
                    NamedKey::ArrowLeft => InputKey::ArrowLeft,
                    NamedKey::ArrowRight => InputKey::ArrowRight,
                    NamedKey::PageUp => InputKey::PageUp,
                    NamedKey::PageDown => InputKey::PageDown,
                    NamedKey::Home => InputKey::Home,
                    NamedKey::End => InputKey::End,
                    NamedKey::Control => InputKey::Control,
                    NamedKey::F5 => InputKey::F5,
                    NamedKey::F9 => InputKey::F9,
                    _ => InputKey::Other,
                }
            }
            Key::Character(value) => InputKey::Character(value.to_string()),
            _ => InputKey::Other,
        },
    )
}
fn button_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

