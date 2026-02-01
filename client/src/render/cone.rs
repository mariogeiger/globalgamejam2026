use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::config::{EYE_HEIGHT, HUNTER_CONE_ALPHA, HUNTER_CONE_LENGTH, TARGETING_ANGLE};
use crate::gpu::uniform_bind_group_layout;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ConeVertex {
    position: [f32; 3],
}

impl ConeVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            }],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct ConeUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}

fn generate_cone_mesh(
    length: f32,
    half_angle_rad: f32,
    segments: u32,
) -> (Vec<ConeVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Apex at origin
    vertices.push(ConeVertex {
        position: [0.0, 0.0, 0.0],
    });

    // Base circle at z = length
    let base_radius = length * half_angle_rad.tan();
    let step = std::f32::consts::TAU / segments as f32;

    for i in 0..segments {
        let angle = step * i as f32;
        let x = base_radius * angle.cos();
        let y = base_radius * angle.sin();
        vertices.push(ConeVertex {
            position: [x, y, length],
        });
    }

    // Side triangles (apex to each edge segment)
    for i in 0..segments {
        let next = (i + 1) % segments;
        indices.push(0);
        indices.push(1 + i);
        indices.push(1 + next);
    }

    // Base cap (optional, but helps with visibility)
    let base_center_idx = vertices.len() as u32;
    vertices.push(ConeVertex {
        position: [0.0, 0.0, length],
    });

    for i in 0..segments {
        let next = (i + 1) % segments;
        indices.push(base_center_idx);
        indices.push(1 + next);
        indices.push(1 + i);
    }

    (vertices, indices)
}

pub struct ConeRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_layout: wgpu::BindGroupLayout,
    uniform_pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

impl ConeRenderer {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cone Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cone.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "Cone Uniform Layout");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cone Pipeline Layout"),
            bind_group_layouts: &[camera_layout, &uniform_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cone Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[ConeVertex::desc()],
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
                depth_write_enabled: false, // Transparent, don't write depth
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();
        let (vertices, indices) = generate_cone_mesh(HUNTER_CONE_LENGTH, half_angle_rad, 32);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cone Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cone Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            uniform_layout,
            uniform_pool: Vec::new(),
        }
    }

    fn ensure_pool_size(&mut self, device: &wgpu::Device, count: usize) {
        while self.uniform_pool.len() < count {
            let uniform = ConeUniform {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [1.0, 0.3, 0.3, HUNTER_CONE_ALPHA],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Cone Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Cone Bind Group"),
                layout: &self.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.uniform_pool.push((buffer, bind_group));
        }
    }

    /// Render vision cones for Hunter mask players
    /// Each cone is defined by (position, yaw, pitch)
    pub fn render<'a>(
        &'a mut self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        camera_bind_group: &'a wgpu::BindGroup,
        cones: &[(Vec3, f32, f32)], // (position, yaw, pitch)
    ) {
        if cones.is_empty() {
            return;
        }

        self.ensure_pool_size(device, cones.len());

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for (i, &(position, yaw, pitch)) in cones.iter().enumerate() {
            // Cone extends in +Z direction in local space, but player looks in -Z
            // So we rotate 180° around Y to flip it forward
            let eye_pos = position + Vec3::new(0.0, EYE_HEIGHT, 0.0);

            // Build model matrix: translate to eye position, rotate to look direction, flip forward
            let model = Mat4::from_translation(eye_pos)
                * Mat4::from_rotation_y(-yaw + std::f32::consts::PI)
                * Mat4::from_rotation_x(-pitch);

            let uniform = ConeUniform {
                model: model.to_cols_array_2d(),
                color: [1.0, 0.3, 0.3, HUNTER_CONE_ALPHA],
            };

            let (buffer, bind_group) = &self.uniform_pool[i];
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[uniform]));
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}
