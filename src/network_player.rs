use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Mat4};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    A,
    B,
}

impl Team {
    pub fn color(&self) -> [f32; 4] {
        match self {
            Team::A => [0.2, 0.4, 1.0, 1.0],  // Blue
            Team::B => [1.0, 0.3, 0.2, 1.0],  // Red
        }
    }
    
    pub fn spawn_points(&self) -> &'static [[f32; 3]] {
        use crate::glb::{SPAWNS_TEAM_A, SPAWNS_TEAM_B};
        match self {
            Team::A => SPAWNS_TEAM_A,
            Team::B => SPAWNS_TEAM_B,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemotePlayer {
    pub position: Vec3,
    pub yaw: f32,
    pub team: Team,
}

impl RemotePlayer {
    pub fn new(team: Team) -> Self {
        let spawns = team.spawn_points();
        const SPAWN_SCALE: f32 = 64.0;
        let spawn = spawns[0];
        let spawn_point = Vec3::new(
            -spawn[0] * SPAWN_SCALE,
            spawn[2] * SPAWN_SCALE,
            spawn[1] * SPAWN_SCALE,
        );
        
        Self {
            position: spawn_point,
            yaw: 0.0,
            team,
        }
    }
    
    pub fn model_matrix(&self) -> Mat4 {
        // Position the player, rotate by yaw
        Mat4::from_translation(self.position)
            * Mat4::from_rotation_y(self.yaw)
    }
}

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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TeamAssignMessage {
    pub msg_type: String,
    pub team: String,
}

// Player mesh vertex
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl PlayerVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PlayerVertex>() as wgpu::BufferAddress,
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

// Generate a simple box mesh for player visualization
pub fn generate_player_box() -> (Vec<PlayerVertex>, Vec<u32>) {
    // Box dimensions: width=32, height=72 (roughly player size), depth=32
    let hw = 16.0;  // half width
    let hd = 16.0;  // half depth
    let h = 72.0;   // full height
    
    let vertices = vec![
        // Front face (Z+)
        PlayerVertex { position: [-hw, 0.0, hd], normal: [0.0, 0.0, 1.0] },
        PlayerVertex { position: [hw, 0.0, hd], normal: [0.0, 0.0, 1.0] },
        PlayerVertex { position: [hw, h, hd], normal: [0.0, 0.0, 1.0] },
        PlayerVertex { position: [-hw, h, hd], normal: [0.0, 0.0, 1.0] },
        
        // Back face (Z-)
        PlayerVertex { position: [hw, 0.0, -hd], normal: [0.0, 0.0, -1.0] },
        PlayerVertex { position: [-hw, 0.0, -hd], normal: [0.0, 0.0, -1.0] },
        PlayerVertex { position: [-hw, h, -hd], normal: [0.0, 0.0, -1.0] },
        PlayerVertex { position: [hw, h, -hd], normal: [0.0, 0.0, -1.0] },
        
        // Left face (X-)
        PlayerVertex { position: [-hw, 0.0, -hd], normal: [-1.0, 0.0, 0.0] },
        PlayerVertex { position: [-hw, 0.0, hd], normal: [-1.0, 0.0, 0.0] },
        PlayerVertex { position: [-hw, h, hd], normal: [-1.0, 0.0, 0.0] },
        PlayerVertex { position: [-hw, h, -hd], normal: [-1.0, 0.0, 0.0] },
        
        // Right face (X+)
        PlayerVertex { position: [hw, 0.0, hd], normal: [1.0, 0.0, 0.0] },
        PlayerVertex { position: [hw, 0.0, -hd], normal: [1.0, 0.0, 0.0] },
        PlayerVertex { position: [hw, h, -hd], normal: [1.0, 0.0, 0.0] },
        PlayerVertex { position: [hw, h, hd], normal: [1.0, 0.0, 0.0] },
        
        // Top face (Y+)
        PlayerVertex { position: [-hw, h, hd], normal: [0.0, 1.0, 0.0] },
        PlayerVertex { position: [hw, h, hd], normal: [0.0, 1.0, 0.0] },
        PlayerVertex { position: [hw, h, -hd], normal: [0.0, 1.0, 0.0] },
        PlayerVertex { position: [-hw, h, -hd], normal: [0.0, 1.0, 0.0] },
        
        // Bottom face (Y-)
        PlayerVertex { position: [-hw, 0.0, -hd], normal: [0.0, -1.0, 0.0] },
        PlayerVertex { position: [hw, 0.0, -hd], normal: [0.0, -1.0, 0.0] },
        PlayerVertex { position: [hw, 0.0, hd], normal: [0.0, -1.0, 0.0] },
        PlayerVertex { position: [-hw, 0.0, hd], normal: [0.0, -1.0, 0.0] },
    ];
    
    let indices = vec![
        // Front
        0, 1, 2, 0, 2, 3,
        // Back
        4, 5, 6, 4, 6, 7,
        // Left
        8, 9, 10, 8, 10, 11,
        // Right
        12, 13, 14, 12, 14, 15,
        // Top
        16, 17, 18, 16, 18, 19,
        // Bottom
        20, 21, 22, 20, 22, 23,
    ];
    
    (vertices, indices)
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}
