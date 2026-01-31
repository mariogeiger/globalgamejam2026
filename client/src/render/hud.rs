use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::config::TARGETING_ANGLE;
use crate::gpu::uniform_bind_group_layout;

/// Distance from camera to place the reticle circle (in world units)
const RETICLE_DISTANCE: f32 = 10.0;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct HudUniform {
    projection: [[f32; 4]; 4],
    targeting_progress: f32,
    has_target: f32,
    reticle_distance: f32,
    reticle_radius: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct HudVertex {
    position: [f32; 2],
}

impl HudVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        }
    }
}

fn generate_circle(segments: u32) -> Vec<HudVertex> {
    let mut vertices = Vec::with_capacity((segments * 3) as usize);
    let step = std::f32::consts::TAU / segments as f32;

    for i in 0..segments {
        let angle1 = step * i as f32;
        let angle2 = step * (i + 1) as f32;

        vertices.push(HudVertex {
            position: [0.0, 0.0],
        });
        vertices.push(HudVertex {
            position: [angle1.cos(), angle1.sin()],
        });
        vertices.push(HudVertex {
            position: [angle2.cos(), angle2.sin()],
        });
    }

    vertices
}

pub struct HudRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl HudRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HUD Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hud.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "HUD Uniform Layout");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD Pipeline Layout"),
            bind_group_layouts: &[&uniform_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[HudVertex::desc()],
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
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let vertices = generate_circle(64);
        let vertex_count = vertices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Calculate reticle radius from targeting angle and distance
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();
        let reticle_radius = RETICLE_DISTANCE * half_angle_rad.tan();

        let uniform = HudUniform {
            projection: Mat4::IDENTITY.to_cols_array_2d(),
            targeting_progress: 0.0,
            has_target: 0.0,
            reticle_distance: RETICLE_DISTANCE,
            reticle_radius,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD Bind Group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            vertex_buffer,
            vertex_count,
            uniform_buffer,
            bind_group,
        }
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        queue: &wgpu::Queue,
        projection: Mat4,
        targeting_progress: f32,
        has_target: bool,
    ) {
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();
        let reticle_radius = RETICLE_DISTANCE * half_angle_rad.tan();

        let uniform = HudUniform {
            projection: projection.to_cols_array_2d(),
            targeting_progress,
            has_target: if has_target { 1.0 } else { 0.0 },
            reticle_distance: RETICLE_DISTANCE,
            reticle_radius,
        };

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}
