use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use wgpu::util::DeviceExt;
use winit::keyboard::KeyCode;

use crate::gpu::uniform_bind_group_layout;

use crate::config::*;
use crate::glb::{SPAWNS_TEAM_A, SPAWNS_TEAM_B};

// === Team ===

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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

        // Full air control - same movement on ground and in air
        self.velocity.x = move_dir.x * MOVE_SPEED;
        self.velocity.z = move_dir.z * MOVE_SPEED;

        // Jump
        if self.on_ground && self.pressed_keys.contains(&KeyCode::Space) {
            self.velocity.y = JUMP_VELOCITY;
            self.on_ground = false;
        }

        // Gravity
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
}

// === Remote Player ===

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
    let hw = PLAYER_WIDTH / 2.0;
    let hd = PLAYER_WIDTH / 2.0;
    let head_height = 2.0 * (PLAYER_HEIGHT - EYE_HEIGHT); // eyes at center of head
    let leg_height = STEP_OVER_HEIGHT;
    let body_top = PLAYER_HEIGHT - head_height;

    let mut vertices = Vec::with_capacity(24 * 3 + 18); // 2 legs + body + head
    let mut indices = Vec::with_capacity(36 * 3 + 24);

    // Helper to add a box
    let mut add_box = |x_min: f32, x_max: f32, y_min: f32, y_max: f32, z_min: f32, z_max: f32| {
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 0.0, 1.0],
                [
                    [x_min, y_min, z_max],
                    [x_max, y_min, z_max],
                    [x_max, y_max, z_max],
                    [x_min, y_max, z_max],
                ],
            ), // +Z
            (
                [0.0, 0.0, -1.0],
                [
                    [x_max, y_min, z_min],
                    [x_min, y_min, z_min],
                    [x_min, y_max, z_min],
                    [x_max, y_max, z_min],
                ],
            ), // -Z
            (
                [-1.0, 0.0, 0.0],
                [
                    [x_min, y_min, z_min],
                    [x_min, y_min, z_max],
                    [x_min, y_max, z_max],
                    [x_min, y_max, z_min],
                ],
            ), // -X
            (
                [1.0, 0.0, 0.0],
                [
                    [x_max, y_min, z_max],
                    [x_max, y_min, z_min],
                    [x_max, y_max, z_min],
                    [x_max, y_max, z_max],
                ],
            ), // +X
            (
                [0.0, 1.0, 0.0],
                [
                    [x_min, y_max, z_max],
                    [x_max, y_max, z_max],
                    [x_max, y_max, z_min],
                    [x_min, y_max, z_min],
                ],
            ), // +Y
            (
                [0.0, -1.0, 0.0],
                [
                    [x_min, y_min, z_min],
                    [x_max, y_min, z_min],
                    [x_max, y_min, z_max],
                    [x_min, y_min, z_max],
                ],
            ), // -Y
        ];
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
    };

    // Legs: two boxes from 0 to leg_height
    let leg_gap = hw * 0.3; // gap between legs
    // Left leg
    add_box(-hw, -leg_gap, 0.0, leg_height, -hd * 0.8, hd * 0.8);
    // Right leg
    add_box(leg_gap, hw, 0.0, leg_height, -hd * 0.8, hd * 0.8);

    // Body (torso): from leg_height to body_top
    add_box(-hw, hw, leg_height, body_top, -hd, hd);

    // Head: triangular prism pointing forward (-Z direction)
    let head_base = body_top; // sits on top of body
    let head_top = PLAYER_HEIGHT; // reaches total height
    let head_tip_z = -hd - PLAYER_WIDTH * 0.6; // tip extends forward

    // Head vertices: back edge (two corners at body top) + front tip
    // Back-left, back-right corners at body height
    let bl = [-hw * 0.7, head_base, hd * 0.5];
    let br = [hw * 0.7, head_base, hd * 0.5];
    let tl = [-hw * 0.7, head_top, hd * 0.5];
    let tr = [hw * 0.7, head_top, hd * 0.5];
    // Front tip (the point)
    let fb = [0.0, head_base, head_tip_z];
    let ft = [0.0, head_top, head_tip_z];

    // Calculate normals for angled faces (pointing outward)
    let left_normal = {
        let edge1 = Vec3::new(tl[0] - bl[0], tl[1] - bl[1], tl[2] - bl[2]); // bl → tl (up)
        let edge2 = Vec3::new(fb[0] - bl[0], fb[1] - bl[1], fb[2] - bl[2]); // bl → fb (forward)
        edge1.cross(edge2).normalize()
    };
    let right_normal = {
        let edge1 = Vec3::new(fb[0] - br[0], fb[1] - br[1], fb[2] - br[2]); // br → fb (forward)
        let edge2 = Vec3::new(tr[0] - br[0], tr[1] - br[1], tr[2] - br[2]); // br → tr (up)
        edge1.cross(edge2).normalize()
    };

    // Left face of head (quad: bl, tl, ft, fb) - CCW from outside (-X)
    let base = vertices.len() as u32;
    for pos in [bl, tl, ft, fb] {
        vertices.push(PlayerVertex {
            position: pos,
            normal: [left_normal.x, left_normal.y, left_normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Right face of head (quad: br, fb, ft, tr) - CCW from outside (+X)
    let base = vertices.len() as u32;
    for pos in [br, fb, ft, tr] {
        vertices.push(PlayerVertex {
            position: pos,
            normal: [right_normal.x, right_normal.y, right_normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Back face of head (quad: bl, br, tr, tl) - CCW from outside (+Z)
    let base = vertices.len() as u32;
    for pos in [bl, br, tr, tl] {
        vertices.push(PlayerVertex {
            position: pos,
            normal: [0.0, 0.0, 1.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Top face of head (triangle: tl, tr, ft) - CCW from outside (+Y)
    let base = vertices.len() as u32;
    for pos in [tl, tr, ft] {
        vertices.push(PlayerVertex {
            position: pos,
            normal: [0.0, 1.0, 0.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2]);

    // Bottom face of head (triangle: bl, fb, br) - CCW from outside (-Y)
    let base = vertices.len() as u32;
    for pos in [bl, fb, br] {
        vertices.push(PlayerVertex {
            position: pos,
            normal: [0.0, -1.0, 0.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2]);

    (vertices, indices)
}

// === Player Renderer ===

pub struct PlayerRenderer {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    uniform_pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

impl PlayerRenderer {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Player Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("player.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "Player Uniform Layout");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Player Pipeline Layout"),
            bind_group_layouts: &[camera_layout, &uniform_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Player Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[PlayerVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (vertices, indices) = generate_player_box();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Player Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Player Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            pipeline,
            uniform_layout,
            uniform_pool: Vec::new(),
        }
    }

    fn ensure_pool_size(&mut self, device: &wgpu::Device, count: usize) {
        while self.uniform_pool.len() < count {
            let uniform = PlayerUniform {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [1.0, 1.0, 1.0, 1.0],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Player Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Player Bind Group"),
                layout: &self.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.uniform_pool.push((buffer, bind_group));
        }
    }

    pub fn render<'a>(
        &'a mut self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        camera_bind_group: &'a wgpu::BindGroup,
        players: &[(Mat4, [f32; 4])],
    ) {
        if players.is_empty() {
            return;
        }

        self.ensure_pool_size(device, players.len());

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for (i, (model, color)) in players.iter().enumerate() {
            let uniform = PlayerUniform {
                model: model.to_cols_array_2d(),
                color: *color,
            };
            let (buffer, bind_group) = &self.uniform_pool[i];
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[uniform]));
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}
