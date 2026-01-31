use glam::{Mat4, Vec3};
use rand::Rng;
use winit::keyboard::KeyCode;

use crate::config::*;
use crate::input::InputState;
use crate::team::Team;

pub struct Player {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub on_ground: bool,
}

impl Player {
    pub fn new(spawn_position: Vec3) -> Self {
        Self {
            position: spawn_position,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            on_ground: false,
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

        self.velocity.x = move_dir.x * MOVE_SPEED;
        self.velocity.z = move_dir.z * MOVE_SPEED;

        if self.on_ground && input.is_pressed(KeyCode::Space) {
            self.velocity.y = JUMP_VELOCITY;
            self.on_ground = false;
        }

        if !self.on_ground {
            self.velocity.y -= GRAVITY * dt;
        }

        self.position += self.velocity * dt;
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
    pub team: Team,
    pub is_alive: bool,
    pub targeted_time: f32,
    pub dead_time: f32,
}

impl RemotePlayer {
    pub fn new(team: Team) -> Self {
        let spawns = team.spawn_points();
        let idx = rand::rng().random_range(0..spawns.len());
        let spawn = spawns[idx];

        Self {
            position: Vec3::new(spawn[0], spawn[1], spawn[2]),
            yaw: 0.0,
            team,
            is_alive: true,
            targeted_time: 0.0,
            dead_time: 0.0,
        }
    }

    pub fn respawn(&mut self) {
        let spawns = self.team.spawn_points();
        let idx = rand::rng().random_range(0..spawns.len());
        let spawn = spawns[idx];
        self.position = Vec3::new(spawn[0], spawn[1], spawn[2]);
        self.is_alive = true;
        self.targeted_time = 0.0;
        self.dead_time = 0.0;
    }

    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position) * Mat4::from_rotation_y(self.yaw)
    }

    pub fn center_mass(&self) -> Vec3 {
        self.position + Vec3::new(0.0, PLAYER_HEIGHT / 2.0, 0.0)
    }
}
