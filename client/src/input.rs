use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    mouse_delta: (f32, f32),
    pub cursor_grabbed: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            pressed_keys: HashSet::new(),
            mouse_delta: (0.0, 0.0),
            cursor_grabbed: false,
        }
    }

    pub fn handle_key_press(&mut self, key: KeyCode) {
        self.pressed_keys.insert(key);
    }

    pub fn handle_key_release(&mut self, key: KeyCode) {
        self.pressed_keys.remove(&key);
    }

    pub fn handle_mouse_move(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    pub fn consume_mouse_delta(&mut self) -> (f32, f32) {
        let delta = self.mouse_delta;
        self.mouse_delta = (0.0, 0.0);
        delta
    }

    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed_keys.contains(&key)
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}
