use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::HashMap;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct MapVertex {
    pub position: [f32; 3],
    pub tex_coord: [f32; 2],
    pub normal: [f32; 3],
}

impl MapVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Float32x3
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MapVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct MapMesh {
    pub vertices: Vec<MapVertex>,
    pub indices: Vec<u32>,
    pub texture_name: String,
}

pub struct LoadedMap {
    pub meshes: Vec<MapMesh>,
    pub textures: HashMap<String, TextureData>,
    pub spawn_points: Vec<Vec3>,
    pub collision_vertices: Vec<Vec3>,
    pub collision_indices: Vec<[u32; 3]>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}
