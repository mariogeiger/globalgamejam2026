use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    mouse_delta: (f32, f32),
    scroll_delta: f32,
    pub cursor_grabbed: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            pressed_keys: HashSet::new(),
            just_pressed_keys: HashSet::new(),
            mouse_delta: (0.0, 0.0),
            scroll_delta: 0.0,
            cursor_grabbed: false,
        }
    }

    pub fn handle_key_press(&mut self, key: KeyCode) {
        if !self.pressed_keys.contains(&key) {
            self.just_pressed_keys.insert(key);
        }
        self.pressed_keys.insert(key);
    }

    pub fn handle_key_release(&mut self, key: KeyCode) {
        self.pressed_keys.remove(&key);
    }

    pub fn handle_mouse_move(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    pub fn handle_scroll(&mut self, delta: f32) {
        self.scroll_delta += delta;
    }

    pub fn consume_mouse_delta(&mut self) -> (f32, f32) {
        let delta = self.mouse_delta;
        self.mouse_delta = (0.0, 0.0);
        delta
    }

    pub fn consume_scroll(&mut self) -> f32 {
        let delta = self.scroll_delta;
        self.scroll_delta = 0.0;
        delta
    }

    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.pressed_keys.contains(&key)
    }

    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.just_pressed_keys.contains(&key)
    }

    pub fn end_frame(&mut self) {
        self.just_pressed_keys.clear();
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}
