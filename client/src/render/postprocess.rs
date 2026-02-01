use bytemuck::{Pod, Zeroable};

use crate::gpu::{
    create_render_target_texture, create_uniform_buffer, create_vertex_buffer,
    texture_bind_group_layout, uniform_bind_group_layout,
};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PostProcessUniform {
    pub resolution: [f32; 2],
    pub time: f32,
    pub _padding: f32,
}

pub struct PostProcessApplyParams {
    pub width: u32,
    pub height: u32,
    pub mask_type: u8,
    pub time: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct FullscreenVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
}

impl FullscreenVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FullscreenVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

const FULLSCREEN_QUAD: [FullscreenVertex; 6] = [
    FullscreenVertex {
        position: [-1.0, -1.0],
        tex_coord: [0.0, 1.0],
    },
    FullscreenVertex {
        position: [1.0, -1.0],
        tex_coord: [1.0, 1.0],
    },
    FullscreenVertex {
        position: [-1.0, 1.0],
        tex_coord: [0.0, 0.0],
    },
    FullscreenVertex {
        position: [1.0, -1.0],
        tex_coord: [1.0, 1.0],
    },
    FullscreenVertex {
        position: [1.0, 1.0],
        tex_coord: [1.0, 0.0],
    },
    FullscreenVertex {
        position: [-1.0, 1.0],
        tex_coord: [0.0, 0.0],
    },
];

#[allow(dead_code)]
pub struct PostProcessor {
    ghost_pipeline: wgpu::RenderPipeline,
    coward_pipeline: wgpu::RenderPipeline,
    hunter_pipeline: wgpu::RenderPipeline,
    offscreen_texture: wgpu::Texture,
    offscreen_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    uniform_bind_group: wgpu::BindGroup,
    quad_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    texture_layout: wgpu::BindGroupLayout,
    uniform_layout: wgpu::BindGroupLayout,
}

impl PostProcessor {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let (offscreen_texture, offscreen_view) =
            create_render_target_texture(device, width, height, surface_format);

        let texture_layout = texture_bind_group_layout(device);
        let uniform_layout = uniform_bind_group_layout(device, "Postprocess Uniform Layout");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Postprocess Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Postprocess Pipeline Layout"),
            bind_group_layouts: &[&texture_layout, &uniform_layout],
            immediate_size: 0,
        });

        let create_pipeline = |shader_src: &str, label: &str| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(shader_src.into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[FullscreenVertex::desc()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: None,
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
            })
        };

        let ghost_pipeline =
            create_pipeline(include_str!("postprocess_ghost.wgsl"), "Ghost Pipeline");
        let coward_pipeline =
            create_pipeline(include_str!("postprocess_coward.wgsl"), "Coward Pipeline");
        let hunter_pipeline =
            create_pipeline(include_str!("postprocess_hunter.wgsl"), "Hunter Pipeline");

        let uniform = PostProcessUniform {
            resolution: [width as f32, height as f32],
            time: 0.0,
            _padding: 0.0,
        };
        let uniform_buffer = create_uniform_buffer(device, &uniform, "Postprocess Uniform");

        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene Bind Group"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&offscreen_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Uniform Bind Group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let quad_buffer = create_vertex_buffer(device, &FULLSCREEN_QUAD, "Fullscreen Quad");

        Self {
            ghost_pipeline,
            coward_pipeline,
            hunter_pipeline,
            offscreen_texture,
            offscreen_view,
            uniform_buffer,
            scene_bind_group,
            uniform_bind_group,
            quad_buffer,
            sampler,
            texture_layout,
            uniform_layout,
        }
    }

    pub fn offscreen_view(&self) -> &wgpu::TextureView {
        &self.offscreen_view
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        let (offscreen_texture, offscreen_view) =
            create_render_target_texture(device, width, height, format);

        self.offscreen_texture = offscreen_texture;
        self.offscreen_view = offscreen_view;

        self.scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scene Bind Group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.offscreen_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
    }

    pub fn apply(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        swapchain_view: &wgpu::TextureView,
        params: PostProcessApplyParams,
    ) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[PostProcessUniform {
                resolution: [params.width as f32, params.height as f32],
                time: params.time,
                _padding: 0.0,
            }]),
        );

        let pipeline = match params.mask_type {
            2 => &self.coward_pipeline,
            3 => &self.hunter_pipeline,
            _ => &self.ghost_pipeline,
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Postprocess Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: swapchain_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.scene_bind_group, &[]);
        pass.set_bind_group(1, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_buffer.slice(..));
        pass.draw(0..6, 0..1);
    }
}
