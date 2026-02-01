use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// Unified vertex type for all meshes
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coord: [f32; 2],
    pub normal: [f32; 3],
}

impl Vertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Float32x3
    ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Texture data (RGBA pixels)
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A submesh with its own material/texture
pub struct SubMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub texture_name: String,
}

/// A loaded mesh with multiple submeshes and textures
pub struct Mesh {
    pub submeshes: Vec<SubMesh>,
    pub textures: HashMap<String, TextureData>,
}

/// Axis-aligned bounding box
#[derive(Clone, Copy, Debug)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl BoundingBox {
    /// Height (Y axis)
    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }
}

impl Mesh {
    /// Calculate the axis-aligned bounding box of all vertices
    pub fn bounding_box(&self) -> BoundingBox {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];

        for submesh in &self.submeshes {
            for v in &submesh.vertices {
                for i in 0..3 {
                    min[i] = min[i].min(v.position[i]);
                    max[i] = max[i].max(v.position[i]);
                }
            }
        }

        BoundingBox { min, max }
    }

    /// Uniformly rescale all vertex positions by the given factor
    pub fn rescale(&mut self, factor: f32) {
        for submesh in &mut self.submeshes {
            for v in &mut submesh.vertices {
                v.position[0] *= factor;
                v.position[1] *= factor;
                v.position[2] *= factor;
            }
        }
    }

    /// Rotate all vertices and normals 180 degrees around the Y axis
    pub fn rotate_y_180(&mut self) {
        for submesh in &mut self.submeshes {
            for v in &mut submesh.vertices {
                // 180 deg rotation: negate X and Z
                v.position[0] = -v.position[0];
                v.position[2] = -v.position[2];
                v.normal[0] = -v.normal[0];
                v.normal[2] = -v.normal[2];
            }
        }
    }

    /// Rotate all vertices and normals 180 degrees around the Z axis (negate X and Y)
    pub fn rotate_z_180(&mut self) {
        for submesh in &mut self.submeshes {
            for v in &mut submesh.vertices {
                v.position[0] = -v.position[0];
                v.position[1] = -v.position[1];
                v.normal[0] = -v.normal[0];
                v.normal[1] = -v.normal[1];
            }
        }
    }
}
