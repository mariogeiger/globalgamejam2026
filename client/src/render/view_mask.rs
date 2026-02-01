// Renders the character's wearable mask in view space (fixed relative to camera).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::gpu::uniform_bind_group_layout;
use crate::mesh::{Mesh, Vertex};

/// Animation: mask moves from FAR to NEAR over DURATION seconds
const MASK_DISTANCE_FAR: f32 = 50.0;
const MASK_DISTANCE_NEAR: f32 = -1.0;
pub const MASK_ANIM_DURATION: f32 = 0.2;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniform {
    projection: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

/// Mask colors: Ghost = brown, Coward = white, Hunter = red
fn mask_color(mask_type: u8) -> [f32; 4] {
    match mask_type {
        1 => [0.6, 0.4, 0.2, 1.0], // Ghost - brown
        2 => [1.0, 1.0, 1.0, 1.0], // Coward - white
        3 => [0.9, 0.1, 0.1, 1.0], // Hunter - red
        _ => [0.5, 0.5, 0.5, 1.0], // fallback - gray
    }
}

pub struct ViewMaskRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl ViewMaskRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, mesh: &Mesh) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("View Mask Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("view_mask.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "View Mask Uniform");

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("View Mask Pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&uniform_layout],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
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

        // Merge all submeshes
        let (vertices, indices): (Vec<_>, Vec<_>) =
            mesh.submeshes
                .iter()
                .fold((Vec::new(), Vec::new()), |(mut verts, mut idxs), sub| {
                    let base = verts.len() as u32;
                    verts.extend_from_slice(&sub.vertices);
                    idxs.extend(sub.indices.iter().map(|i| base + i));
                    (verts, idxs)
                });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("View Mask VB"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("View Mask IB"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("View Mask UB"),
            contents: bytemuck::cast_slice(&[Uniform {
                projection: Mat4::IDENTITY.to_cols_array_2d(),
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [1.0, 1.0, 1.0, 1.0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("View Mask BG"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            uniform_buffer,
            bind_group,
        }
    }

    /// Render mask with animation progress (0.0 = far, 1.0 = near) and mask type for color.
    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        queue: &wgpu::Queue,
        projection: Mat4,
        progress: f32,
        mask_type: u8,
    ) {
        let t = progress.clamp(0.0, 1.0);
        let distance = MASK_DISTANCE_FAR + t * (MASK_DISTANCE_NEAR - MASK_DISTANCE_FAR);

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniform {
                projection: projection.to_cols_array_2d(),
                model: Mat4::from_translation(Vec3::new(0.0, 0.0, -distance)).to_cols_array_2d(),
                color: mask_color(mask_type),
            }]),
        );

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
