use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::config::THREAT_ARROW_ALPHA;
use crate::gpu::uniform_bind_group_layout;

/// Maximum number of threat indicators that can be rendered at once
const MAX_THREATS: usize = 8;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ThreatUniform {
    screen_size: [f32; 2], // offset 0, 8 bytes
    arrow_angle: f32,      // offset 8, 4 bytes
    arrow_alpha: f32,      // offset 12, 4 bytes
    time: f32,             // offset 16, 4 bytes
    _pad1: f32,            // offset 20, 4 bytes
    _pad2: f32,            // offset 24, 4 bytes
    _pad3: f32,            // offset 28, 4 bytes
                           // Total: 32 bytes
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ThreatVertex {
    position: [f32; 2],
}

impl ThreatVertex {
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

/// Generate an arrow shape pointing to the right (+X direction)
/// The arrow will be rotated in the shader based on threat direction
fn generate_arrow() -> Vec<ThreatVertex> {
    // Arrow pointing right, centered at origin
    // Triangle pointing right with a tail
    vec![
        // Arrow head (triangle pointing right)
        ThreatVertex {
            position: [1.0, 0.0],
        }, // Tip
        ThreatVertex {
            position: [0.3, 0.5],
        }, // Top
        ThreatVertex {
            position: [0.3, -0.5],
        }, // Bottom
        // Arrow tail (rectangle)
        ThreatVertex {
            position: [0.3, 0.25],
        },
        ThreatVertex {
            position: [-0.5, 0.25],
        },
        ThreatVertex {
            position: [-0.5, -0.25],
        },
        ThreatVertex {
            position: [0.3, 0.25],
        },
        ThreatVertex {
            position: [-0.5, -0.25],
        },
        ThreatVertex {
            position: [0.3, -0.25],
        },
    ]
}

pub struct ThreatIndicatorRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    #[allow(dead_code)]
    uniform_layout: wgpu::BindGroupLayout,
    uniform_pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

impl ThreatIndicatorRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Threat Indicator Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("threat.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "Threat Uniform Layout");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Threat Pipeline Layout"),
            bind_group_layouts: &[&uniform_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Threat Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[ThreatVertex::desc()],
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

        let vertices = generate_arrow();
        let vertex_count = vertices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Threat Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Pre-allocate uniform buffers for multiple threats
        let mut uniform_pool = Vec::with_capacity(MAX_THREATS);
        for i in 0..MAX_THREATS {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("Threat Uniform Buffer {}", i)),
                contents: bytemuck::cast_slice(&[ThreatUniform {
                    screen_size: [1.0, 1.0],
                    arrow_angle: 0.0,
                    arrow_alpha: 0.0,
                    time: 0.0,
                    _pad1: 0.0,
                    _pad2: 0.0,
                    _pad3: 0.0,
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("Threat Bind Group {}", i)),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });

            uniform_pool.push((buffer, bind_group));
        }

        Self {
            pipeline,
            vertex_buffer,
            vertex_count,
            uniform_layout,
            uniform_pool,
        }
    }

    /// Render threat indicators
    /// `threat_angles` contains the screen-space angle for each threat (in radians)
    /// `time` is the game time for animation
    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        queue: &wgpu::Queue,
        screen_width: f32,
        screen_height: f32,
        threat_angles: &[f32],
        time: f32,
    ) {
        if threat_angles.is_empty() {
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

        for (i, &angle) in threat_angles.iter().take(MAX_THREATS).enumerate() {
            let uniform = ThreatUniform {
                screen_size: [screen_width, screen_height],
                arrow_angle: angle,
                arrow_alpha: THREAT_ARROW_ALPHA,
                time,
                _pad1: 0.0,
                _pad2: 0.0,
                _pad3: 0.0,
            };

            queue.write_buffer(&self.uniform_pool[i].0, 0, bytemuck::cast_slice(&[uniform]));
            pass.set_bind_group(0, &self.uniform_pool[i].1, &[]);
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}
