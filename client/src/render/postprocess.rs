use bytemuck::{Pod, Zeroable};

use crate::gpu::{
    create_render_target_texture, create_uniform_buffer, create_vertex_buffer,
    depth_texture_bind_group_layout, texture_bind_group_layout, uniform_bind_group_layout,
};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PostProcessUniform {
    pub blur_direction: [f32; 2],
    pub blur_strength: f32,
    pub smear_factor: f32,
    pub resolution: [f32; 2],
    pub depth_near: f32,
    pub depth_far: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PostProcessVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
}

impl PostProcessVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PostProcessVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as u64,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

const FULLSCREEN_QUAD: [PostProcessVertex; 6] = [
    PostProcessVertex {
        position: [-1.0, -1.0],
        tex_coord: [0.0, 1.0],
    },
    PostProcessVertex {
        position: [1.0, -1.0],
        tex_coord: [1.0, 1.0],
    },
    PostProcessVertex {
        position: [-1.0, 1.0],
        tex_coord: [0.0, 0.0],
    },
    PostProcessVertex {
        position: [1.0, -1.0],
        tex_coord: [1.0, 1.0],
    },
    PostProcessVertex {
        position: [1.0, 1.0],
        tex_coord: [1.0, 0.0],
    },
    PostProcessVertex {
        position: [-1.0, 1.0],
        tex_coord: [0.0, 0.0],
    },
];

#[allow(dead_code)]
pub struct PostProcessor {
    postprocess_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    offscreen_texture: wgpu::Texture,
    offscreen_view: wgpu::TextureView,
    history_textures: [wgpu::Texture; 2],
    history_views: [wgpu::TextureView; 2],
    history_index: usize,
    uniform_buffer: wgpu::Buffer,
    postprocess_bind_group: wgpu::BindGroup,
    postprocess_uniform_bind_group: wgpu::BindGroup,
    postprocess_previous_bind_groups: [wgpu::BindGroup; 2],
    postprocess_depth_bind_group: wgpu::BindGroup,
    present_bind_groups: [wgpu::BindGroup; 2],
    fullscreen_quad_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    depth_sampler: wgpu::Sampler,
    texture_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    uniform_layout: wgpu::BindGroupLayout,
    pub first_frame: bool,
}

#[allow(dead_code)]
impl PostProcessor {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        depth_view: &wgpu::TextureView,
    ) -> Self {
        let (offscreen_texture, offscreen_view) =
            create_render_target_texture(device, width, height, surface_format);
        let (history_texture_0, history_view_0) =
            create_render_target_texture(device, width, height, surface_format);
        let (history_texture_1, history_view_1) =
            create_render_target_texture(device, width, height, surface_format);

        let texture_layout = texture_bind_group_layout(device);
        let depth_layout = depth_texture_bind_group_layout(device);
        let uniform_layout = uniform_bind_group_layout(device, "Postprocess Uniform Layout");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Postprocess Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Postprocess Depth Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Postprocess Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("postprocess.wgsl").into()),
        });

        let postprocess_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Postprocess Pipeline Layout"),
                bind_group_layouts: &[
                    &texture_layout,
                    &uniform_layout,
                    &texture_layout,
                    &depth_layout,
                ],
                immediate_size: 0,
            });

        let postprocess_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Postprocess Pipeline"),
            layout: Some(&postprocess_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &postprocess_shader,
                entry_point: Some("vs_main"),
                buffers: &[PostProcessVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &postprocess_shader,
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
        });

        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Present Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });

        let present_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Present Pipeline Layout"),
                bind_group_layouts: &[&texture_layout],
                immediate_size: 0,
            });

        let present_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Present Pipeline"),
            layout: Some(&present_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &present_shader,
                entry_point: Some("vs_main"),
                buffers: &[PostProcessVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &present_shader,
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
        });

        let uniform = PostProcessUniform {
            blur_direction: [0.0, 0.0],
            blur_strength: 0.0,
            smear_factor: 0.85,
            resolution: [width as f32, height as f32],
            depth_near: 1.0,
            depth_far: 10000.0,
        };
        let uniform_buffer = create_uniform_buffer(device, &uniform, "Postprocess Uniform");

        let postprocess_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Bind Group (scene)"),
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

        let postprocess_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Uniform Bind Group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let postprocess_previous_bind_group_0 =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 0"),
                layout: &texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&history_view_0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

        let postprocess_previous_bind_group_1 =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 1"),
                layout: &texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&history_view_1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

        let postprocess_depth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Depth Bind Group"),
            layout: &depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&depth_sampler),
                },
            ],
        });

        let present_bind_group_0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Present Bind Group 0"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&history_view_0),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let present_bind_group_1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Present Bind Group 1"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&history_view_1),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let fullscreen_quad_buffer =
            create_vertex_buffer(device, &FULLSCREEN_QUAD, "Fullscreen Quad");

        Self {
            postprocess_pipeline,
            present_pipeline,
            offscreen_texture,
            offscreen_view,
            history_textures: [history_texture_0, history_texture_1],
            history_views: [history_view_0, history_view_1],
            history_index: 0,
            uniform_buffer,
            postprocess_bind_group,
            postprocess_uniform_bind_group,
            postprocess_previous_bind_groups: [
                postprocess_previous_bind_group_0,
                postprocess_previous_bind_group_1,
            ],
            postprocess_depth_bind_group,
            present_bind_groups: [present_bind_group_0, present_bind_group_1],
            fullscreen_quad_buffer,
            sampler,
            depth_sampler,
            texture_layout,
            depth_layout,
            uniform_layout,
            first_frame: true,
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
        depth_view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let (offscreen_texture, offscreen_view) =
            create_render_target_texture(device, width, height, format);
        let (history_texture_0, history_view_0) =
            create_render_target_texture(device, width, height, format);
        let (history_texture_1, history_view_1) =
            create_render_target_texture(device, width, height, format);

        self.offscreen_texture = offscreen_texture;
        self.offscreen_view = offscreen_view;
        self.history_textures = [history_texture_0, history_texture_1];
        self.history_views = [history_view_0, history_view_1];

        self.postprocess_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Bind Group (scene)"),
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

        self.postprocess_depth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Depth Bind Group"),
            layout: &self.depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.depth_sampler),
                },
            ],
        });

        self.postprocess_previous_bind_groups = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 0"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 1"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
        ];

        self.present_bind_groups = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Present Bind Group 0"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Present Bind Group 1"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
        ];

        self.first_frame = true;
    }

    pub fn apply(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        swapchain_view: &wgpu::TextureView,
        velocity: glam::Vec3,
        width: u32,
        height: u32,
    ) {
        let vel_xz = glam::Vec2::new(velocity.x, velocity.z);
        let speed = vel_xz.length();
        let (blur_direction, blur_strength) = if speed > 1.0 {
            let dir = vel_xz.normalize();
            let strength = (speed / 300.0) * 0.02f32;
            ([dir.x, dir.y], strength)
        } else {
            ([0.0f32, 0.0], 0.0)
        };
        let smear_factor = if self.first_frame { 0.0 } else { 0.85 };
        self.first_frame = false;

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[PostProcessUniform {
                blur_direction,
                blur_strength,
                smear_factor,
                resolution: [width as f32, height as f32],
                depth_near: 1.0,
                depth_far: 10000.0,
            }]),
        );

        let prev_idx = self.history_index;
        let next_idx = 1 - prev_idx;
        let history_next_view = &self.history_views[next_idx];
        let postprocess_previous_bind_group = &self.postprocess_previous_bind_groups[prev_idx];
        let present_bind_group = &self.present_bind_groups[next_idx];

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Postprocess Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: history_next_view,
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

            pass.set_pipeline(&self.postprocess_pipeline);
            pass.set_bind_group(0, &self.postprocess_bind_group, &[]);
            pass.set_bind_group(1, &self.postprocess_uniform_bind_group, &[]);
            pass.set_bind_group(2, postprocess_previous_bind_group, &[]);
            pass.set_bind_group(3, &self.postprocess_depth_bind_group, &[]);
            pass.set_vertex_buffer(0, self.fullscreen_quad_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Present Pass"),
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

            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, present_bind_group, &[]);
            pass.set_vertex_buffer(0, self.fullscreen_quad_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }

        self.history_index = next_idx;
    }
}
