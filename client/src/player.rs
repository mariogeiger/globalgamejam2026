use glam::{Mat4, Vec3};
use winit::keyboard::KeyCode;

use crate::config::*;
use crate::input::InputState;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(u8)]
pub enum MaskType {
    #[default]
    Ghost = 1,
    Coward = 2,
    Hunter = 3,
}

impl MaskType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            2 => Self::Coward,
            3 => Self::Hunter,
            _ => Self::Ghost,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Ghost => Self::Coward,
            Self::Coward => Self::Hunter,
            Self::Hunter => Self::Ghost,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Ghost => Self::Hunter,
            Self::Coward => Self::Ghost,
            Self::Hunter => Self::Coward,
        }
    }
}

pub struct Player {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub mask: MaskType,
    pub last_mask: MaskType,
}

impl Player {
    pub fn new(spawn_position: Vec3) -> Self {
        Self {
            position: spawn_position,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            on_ground: false,
            mask: MaskType::Ghost,
            last_mask: MaskType::Ghost,
        }
    }

    pub fn set_mask(&mut self, mask: MaskType) {
        if mask != self.mask {
            self.last_mask = self.mask;
            self.mask = mask;
        }
    }

    pub fn swap_to_last_mask(&mut self) {
        std::mem::swap(&mut self.mask, &mut self.last_mask);
    }

    pub fn cycle_mask_next(&mut self) {
        self.set_mask(self.mask.next());
    }

    pub fn cycle_mask_prev(&mut self) {
        self.set_mask(self.mask.prev());
    }

    pub fn move_speed(&self) -> f32 {
        match self.mask {
            MaskType::Coward => MOVE_SPEED * COWARD_SPEED_MULTIPLIER,
            _ => MOVE_SPEED,
        }
    }

    pub fn update(&mut self, dt: f32, input: &mut InputState) {
        let (dx, dy) = input.consume_mouse_delta();
        self.yaw += dx * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY).clamp(-1.5, 1.5);

        let (sin, cos) = (self.yaw.sin(), self.yaw.cos());
        let forward = Vec3::new(sin, 0.0, -cos);
        let right = Vec3::new(cos, 0.0, sin);

        let mut move_dir = Vec3::ZERO;
        if input.is_pressed(KeyCode::KeyW) {
            move_dir += forward;
        }
        if input.is_pressed(KeyCode::KeyS) {
            move_dir -= forward;
        }
        if input.is_pressed(KeyCode::KeyD) {
            move_dir += right;
        }
        if input.is_pressed(KeyCode::KeyA) {
            move_dir -= right;
        }

        let move_dir = move_dir.normalize_or_zero();
        let speed = self.move_speed();

        self.velocity.x = move_dir.x * speed;
        self.velocity.z = move_dir.z * speed;

        if self.on_ground && input.is_pressed(KeyCode::Space) {
            self.velocity.y = JUMP_VELOCITY;
            self.on_ground = false;

            // Coward gets a directional jump boost
            if self.mask == MaskType::Coward && move_dir != Vec3::ZERO {
                self.velocity.x += move_dir.x * COWARD_JUMP_BOOST;
                self.velocity.z += move_dir.z * COWARD_JUMP_BOOST;
            }
        }

        if !self.on_ground {
            self.velocity.y -= GRAVITY * dt;
        }

        self.position += self.velocity * dt;
    }

    /// Spectator mode: fly freely, 10x speed, no collision
    pub fn spectator_update(&mut self, dt: f32, input: &mut InputState) {
        let (dx, dy) = input.consume_mouse_delta();
        self.yaw += dx * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy * MOUSE_SENSITIVITY).clamp(-1.5, 1.5);

        // 3D movement in look direction
        let look_dir = self.look_direction();
        let right = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());

        let mut move_dir = Vec3::ZERO;
        if input.is_pressed(KeyCode::KeyW) {
            move_dir += look_dir;
        }
        if input.is_pressed(KeyCode::KeyS) {
            move_dir -= look_dir;
        }
        if input.is_pressed(KeyCode::KeyD) {
            move_dir += right;
        }
        if input.is_pressed(KeyCode::KeyA) {
            move_dir -= right;
        }
        if input.is_pressed(KeyCode::Space) {
            move_dir.y += 1.0;
        }
        if input.is_pressed(KeyCode::ShiftLeft) {
            move_dir.y -= 1.0;
        }

        let move_dir = move_dir.normalize_or_zero();
        let spectator_speed = MOVE_SPEED * 10.0;

        self.position += move_dir * spectator_speed * dt;
    }

    pub fn view_matrix(&self) -> Mat4 {
        let eye = self.position + Vec3::new(0.0, EYE_HEIGHT, 0.0);
        let look_dir = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
        .normalize();
        Mat4::look_at_rh(eye, eye + look_dir, Vec3::Y)
    }

    pub fn set_on_ground(&mut self, on_ground: bool, ground_y: Option<f32>) {
        if on_ground && !self.on_ground {
            self.velocity.y = 0.0;
        }
        self.on_ground = on_ground;
        if let Some(y) = ground_y
            && self.on_ground
        {
            self.position.y = y;
        }
    }

    pub fn respawn(&mut self, spawn_position: Vec3) {
        self.position = spawn_position;
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
    }

    pub fn look_direction(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        )
        .normalize()
    }

    pub fn eye_position(&self) -> Vec3 {
        self.position + Vec3::new(0.0, EYE_HEIGHT, 0.0)
    }
}

pub struct RemotePlayer {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub is_alive: bool,
    pub targeted_time: f32,
    pub mask: MaskType,
    pub velocity: Vec3,
    prev_position: Vec3,
}

impl RemotePlayer {
    pub fn new() -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            is_alive: true,
            targeted_time: 0.0,
            mask: MaskType::Ghost,
            velocity: Vec3::ZERO,
            prev_position: Vec3::ZERO,
        }
    }

    pub fn update_position(&mut self, new_position: Vec3, dt: f32) {
        if dt > 0.0 {
            self.velocity = (new_position - self.prev_position) / dt;
        }
        self.prev_position = self.position;
        self.position = new_position;
    }

    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position) * Mat4::from_rotation_y(-self.yaw)
    }

    pub fn center_mass(&self) -> Vec3 {
        self.position + Vec3::new(0.0, PLAYER_HEIGHT / 2.0, 0.0)
    }
}
