use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

use crate::config::*;
use crate::glb::{SPAWNS_TEAM_A, SPAWNS_TEAM_B};

// === Team ===

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    A,
    B,
}

impl Team {
    pub fn color(&self) -> [f32; 4] {
        match self {
            Team::A => [0.2, 0.4, 1.0, 1.0],
            Team::B => [1.0, 0.3, 0.2, 1.0],
        }
    }

    pub fn spawn_points(&self) -> &'static [[f32; 3]] {
        match self {
            Team::A => SPAWNS_TEAM_A,
            Team::B => SPAWNS_TEAM_B,
        }
    }
}

// === Local Player ===

pub struct Player {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub on_ground: bool,
    pressed_keys: HashSet<KeyCode>,
    mouse_delta: (f32, f32),
}

impl Player {
    pub fn new(spawn_position: Vec3) -> Self {
        Self {
            position: spawn_position,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            on_ground: false,
            pressed_keys: HashSet::new(),
            mouse_delta: (0.0, 0.0),
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

    pub fn update(&mut self, dt: f32) {
        // Mouse look
        self.yaw += self.mouse_delta.0 * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - self.mouse_delta.1 * MOUSE_SENSITIVITY).clamp(-1.5, 1.5);
        self.mouse_delta = (0.0, 0.0);

        // Movement vectors
        let (sin, cos) = (self.yaw.sin(), self.yaw.cos());
        let forward = Vec3::new(sin, 0.0, -cos);
        let right = Vec3::new(cos, 0.0, sin);

        // Build movement direction from pressed keys
        let key = |k| self.pressed_keys.contains(&k);
        let move_dir = Vec3::ZERO
            + if key(KeyCode::KeyW) {
                forward
            } else {
                Vec3::ZERO
            }
            - if key(KeyCode::KeyS) {
                forward
            } else {
                Vec3::ZERO
            }
            + if key(KeyCode::KeyD) {
                right
            } else {
                Vec3::ZERO
            }
            - if key(KeyCode::KeyA) {
                right
            } else {
                Vec3::ZERO
            };

        let move_dir = move_dir.normalize_or_zero();

        if self.on_ground {
            self.velocity.x = move_dir.x * MOVE_SPEED;
            self.velocity.z = move_dir.z * MOVE_SPEED;
            if self.pressed_keys.contains(&KeyCode::Space) {
                self.velocity.y = JUMP_VELOCITY;
                self.on_ground = false;
            }
        } else {
            self.velocity.x += move_dir.x * MOVE_SPEED * 0.1 * dt;
            self.velocity.z += move_dir.z * MOVE_SPEED * 0.1 * dt;
        }

        if !self.on_ground {
            self.velocity.y -= GRAVITY * dt;
        }

        self.position += self.velocity * dt;

        if self.on_ground {
            let friction = (1.0 - FRICTION * dt).max(0.0);
            self.velocity.x *= friction;
            self.velocity.z *= friction;
        }
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
        if let Some(y) = ground_y {
            if self.on_ground {
                self.position.y = y;
            }
        }
    }

    pub fn respawn(&mut self, spawn_position: Vec3) {
        self.position = spawn_position;
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
    }
}

// === Remote Player ===

#[derive(Clone, Debug)]
pub struct RemotePlayer {
    pub position: Vec3,
    pub yaw: f32,
    pub team: Team,
}

impl RemotePlayer {
    pub fn new(team: Team) -> Self {
        let spawn = team.spawn_points()[0];
        Self {
            position: Vec3::new(
                -spawn[0] * SPAWN_SCALE,
                spawn[2] * SPAWN_SCALE,
                spawn[1] * SPAWN_SCALE,
            ),
            yaw: 0.0,
            team,
        }
    }

    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.position) * Mat4::from_rotation_y(self.yaw)
    }
}

// === Network Message ===

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerStateMessage {
    pub msg_type: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
}

impl PlayerStateMessage {
    pub fn new(position: Vec3, yaw: f32) -> Self {
        Self {
            msg_type: "player_state".to_string(),
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
        }
    }
}

// === GPU Vertex/Uniform Types ===

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl PlayerVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}

pub fn generate_player_box() -> (Vec<PlayerVertex>, Vec<u32>) {
    let (hw, hd, h) = (PLAYER_WIDTH / 2.0, PLAYER_WIDTH / 2.0, PLAYER_HEIGHT);

    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [[-hw, 0.0, hd], [hw, 0.0, hd], [hw, h, hd], [-hw, h, hd]],
        ), // Front
        (
            [0.0, 0.0, -1.0],
            [[hw, 0.0, -hd], [-hw, 0.0, -hd], [-hw, h, -hd], [hw, h, -hd]],
        ), // Back
        (
            [-1.0, 0.0, 0.0],
            [[-hw, 0.0, -hd], [-hw, 0.0, hd], [-hw, h, hd], [-hw, h, -hd]],
        ), // Left
        (
            [1.0, 0.0, 0.0],
            [[hw, 0.0, hd], [hw, 0.0, -hd], [hw, h, -hd], [hw, h, hd]],
        ), // Right
        (
            [0.0, 1.0, 0.0],
            [[-hw, h, hd], [hw, h, hd], [hw, h, -hd], [-hw, h, -hd]],
        ), // Top
        (
            [0.0, -1.0, 0.0],
            [
                [-hw, 0.0, -hd],
                [hw, 0.0, -hd],
                [hw, 0.0, hd],
                [-hw, 0.0, hd],
            ],
        ), // Bottom
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);

    for (normal, positions) in faces {
        let base = vertices.len() as u32;
        for pos in positions {
            vertices.push(PlayerVertex {
                position: pos,
                normal,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    (vertices, indices)
}
