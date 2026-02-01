use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::gpu::{
    create_texture_with_bind_group, texture_bind_group_layout, uniform_bind_group_layout,
};
use crate::mesh::{Mesh, Vertex};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}

/// Merge all submeshes into single vertex/index buffers
fn merge_submeshes(mesh: &Mesh) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for submesh in &mesh.submeshes {
        let base_idx = vertices.len() as u32;
        vertices.extend_from_slice(&submesh.vertices);
        indices.extend(submesh.indices.iter().map(|i| base_idx + i));
    }

    (vertices, indices)
}

pub struct PlayerRenderer {
    player_vertex_buffer: wgpu::Buffer,
    player_index_buffer: wgpu::Buffer,
    player_index_count: u32,
    player_texture_bind_group: wgpu::BindGroup,
    tombstone_vertex_buffer: wgpu::Buffer,
    tombstone_index_buffer: wgpu::Buffer,
    tombstone_index_count: u32,
    tombstone_texture_bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    uniform_pool: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

impl PlayerRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        player_mesh: &Mesh,
        tombstone_mesh: &Mesh,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Player Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("player.wgsl").into()),
        });

        let uniform_layout = uniform_bind_group_layout(device, "Player Uniform Layout");
        let texture_layout = texture_bind_group_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Player Pipeline Layout"),
            bind_group_layouts: &[camera_layout, &uniform_layout, &texture_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Player Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
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

        let (player_vertices, player_indices) = merge_submeshes(player_mesh);

        let player_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Player Vertex Buffer"),
            contents: bytemuck::cast_slice(&player_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let player_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Player Index Buffer"),
            contents: bytemuck::cast_slice(&player_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Merge tombstone submeshes into single buffer
        let (tombstone_vertices, tombstone_indices) = merge_submeshes(tombstone_mesh);

        let tombstone_vertex_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Tombstone Vertex Buffer"),
                contents: bytemuck::cast_slice(&tombstone_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let tombstone_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tombstone Index Buffer"),
            contents: bytemuck::cast_slice(&tombstone_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create sampler for textures
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create player texture (or white fallback)
        let white_pixel: [u8; 4] = [255, 255, 255, 255];
        let player_texture_bind_group = if let Some(tex) = player_mesh.textures.values().next() {
            let (_, _, bind_group) = create_texture_with_bind_group(
                device,
                queue,
                &texture_layout,
                &sampler,
                &tex.rgba,
                tex.width,
                tex.height,
                "Player Texture",
            );
            bind_group
        } else {
            let (_, _, bind_group) = create_texture_with_bind_group(
                device,
                queue,
                &texture_layout,
                &sampler,
                &white_pixel,
                1,
                1,
                "Player White Texture",
            );
            bind_group
        };

        // Create tombstone texture (or white fallback)
        let tombstone_texture_bind_group =
            if let Some(tex) = tombstone_mesh.textures.values().next() {
                let (_, _, bind_group) = create_texture_with_bind_group(
                    device,
                    queue,
                    &texture_layout,
                    &sampler,
                    &tex.rgba,
                    tex.width,
                    tex.height,
                    "Tombstone Texture",
                );
                bind_group
            } else {
                let (_, _, bind_group) = create_texture_with_bind_group(
                    device,
                    queue,
                    &texture_layout,
                    &sampler,
                    &white_pixel,
                    1,
                    1,
                    "Tombstone Fallback Texture",
                );
                bind_group
            };

        Self {
            player_vertex_buffer,
            player_index_buffer,
            player_index_count: player_indices.len() as u32,
            player_texture_bind_group,
            tombstone_vertex_buffer,
            tombstone_index_buffer,
            tombstone_index_count: tombstone_indices.len() as u32,
            tombstone_texture_bind_group,
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
        alive_players: &[(Mat4, [f32; 4])],
        dead_players: &[(Mat4, [f32; 4])],
    ) {
        let total_count = alive_players.len() + dead_players.len();
        if total_count == 0 {
            return;
        }

        self.ensure_pool_size(device, total_count);

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);

        // Render alive players with player mesh
        if !alive_players.is_empty() {
            pass.set_vertex_buffer(0, self.player_vertex_buffer.slice(..));
            pass.set_index_buffer(
                self.player_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.set_bind_group(2, &self.player_texture_bind_group, &[]);

            for (i, (model, color)) in alive_players.iter().enumerate() {
                let uniform = PlayerUniform {
                    model: model.to_cols_array_2d(),
                    color: *color,
                };
                let (buffer, bind_group) = &self.uniform_pool[i];
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[uniform]));
                pass.set_bind_group(1, bind_group, &[]);
                pass.draw_indexed(0..self.player_index_count, 0, 0..1);
            }
        }

        // Render dead players with tombstone mesh
        if !dead_players.is_empty() {
            pass.set_vertex_buffer(0, self.tombstone_vertex_buffer.slice(..));
            pass.set_index_buffer(
                self.tombstone_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.set_bind_group(2, &self.tombstone_texture_bind_group, &[]);

            for (i, (model, color)) in dead_players.iter().enumerate() {
                let uniform = PlayerUniform {
                    model: model.to_cols_array_2d(),
                    color: *color,
                };
                let pool_idx = alive_players.len() + i;
                let (buffer, bind_group) = &self.uniform_pool[pool_idx];
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[uniform]));
                pass.set_bind_group(1, bind_group, &[]);
                pass.draw_indexed(0..self.tombstone_index_count, 0, 0..1);
            }
        }
    }
}
