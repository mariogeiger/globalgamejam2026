use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::config::*;
use crate::gpu::{
    create_texture_with_bind_group, texture_bind_group_layout, uniform_bind_group_layout,
};
use crate::mesh::{TextureData, Vertex};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PlayerUniform {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
}

pub fn generate_player_box() -> (Vec<Vertex>, Vec<u32>) {
    let hw = PLAYER_WIDTH / 2.0;
    let hd = PLAYER_WIDTH / 2.0;
    let head_height = 2.0 * (PLAYER_HEIGHT - EYE_HEIGHT);
    let leg_height = STEP_OVER_HEIGHT;
    let body_top = PLAYER_HEIGHT - head_height;

    let mut vertices = Vec::with_capacity(24 * 3 + 18);
    let mut indices = Vec::with_capacity(36 * 3 + 24);

    let mut add_box = |x_min: f32, x_max: f32, y_min: f32, y_max: f32, z_min: f32, z_max: f32| {
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 0.0, 1.0],
                [
                    [x_min, y_min, z_max],
                    [x_max, y_min, z_max],
                    [x_max, y_max, z_max],
                    [x_min, y_max, z_max],
                ],
            ),
            (
                [0.0, 0.0, -1.0],
                [
                    [x_max, y_min, z_min],
                    [x_min, y_min, z_min],
                    [x_min, y_max, z_min],
                    [x_max, y_max, z_min],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                [
                    [x_min, y_min, z_min],
                    [x_min, y_min, z_max],
                    [x_min, y_max, z_max],
                    [x_min, y_max, z_min],
                ],
            ),
            (
                [1.0, 0.0, 0.0],
                [
                    [x_max, y_min, z_max],
                    [x_max, y_min, z_min],
                    [x_max, y_max, z_min],
                    [x_max, y_max, z_max],
                ],
            ),
            (
                [0.0, 1.0, 0.0],
                [
                    [x_min, y_max, z_max],
                    [x_max, y_max, z_max],
                    [x_max, y_max, z_min],
                    [x_min, y_max, z_min],
                ],
            ),
            (
                [0.0, -1.0, 0.0],
                [
                    [x_min, y_min, z_min],
                    [x_max, y_min, z_min],
                    [x_max, y_min, z_max],
                    [x_min, y_min, z_max],
                ],
            ),
        ];
        for (normal, positions) in faces {
            let base = vertices.len() as u32;
            // Simple UV mapping for each face quad
            let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            for (pos, uv) in positions.iter().zip(uvs.iter()) {
                vertices.push(Vertex {
                    position: *pos,
                    tex_coord: *uv,
                    normal,
                });
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    };

    let leg_gap = hw * 0.3;
    add_box(-hw, -leg_gap, 0.0, leg_height, -hd * 0.8, hd * 0.8);
    add_box(leg_gap, hw, 0.0, leg_height, -hd * 0.8, hd * 0.8);
    add_box(-hw, hw, leg_height, body_top, -hd, hd);

    let head_base = body_top;
    let head_top = PLAYER_HEIGHT;
    let head_tip_z = -hd - PLAYER_WIDTH * 0.6;

    let bl = [-hw * 0.7, head_base, hd * 0.5];
    let br = [hw * 0.7, head_base, hd * 0.5];
    let tl = [-hw * 0.7, head_top, hd * 0.5];
    let tr = [hw * 0.7, head_top, hd * 0.5];
    let fb = [0.0, head_base, head_tip_z];
    let ft = [0.0, head_top, head_tip_z];

    let left_normal = {
        let edge1 = Vec3::new(tl[0] - bl[0], tl[1] - bl[1], tl[2] - bl[2]);
        let edge2 = Vec3::new(fb[0] - bl[0], fb[1] - bl[1], fb[2] - bl[2]);
        edge1.cross(edge2).normalize()
    };
    let right_normal = {
        let edge1 = Vec3::new(fb[0] - br[0], fb[1] - br[1], fb[2] - br[2]);
        let edge2 = Vec3::new(tr[0] - br[0], tr[1] - br[1], tr[2] - br[2]);
        edge1.cross(edge2).normalize()
    };

    // Head left face
    let base = vertices.len() as u32;
    let uvs4 = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (pos, uv) in [bl, tl, ft, fb].iter().zip(uvs4.iter()) {
        vertices.push(Vertex {
            position: *pos,
            tex_coord: *uv,
            normal: [left_normal.x, left_normal.y, left_normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Head right face
    let base = vertices.len() as u32;
    for (pos, uv) in [br, fb, ft, tr].iter().zip(uvs4.iter()) {
        vertices.push(Vertex {
            position: *pos,
            tex_coord: *uv,
            normal: [right_normal.x, right_normal.y, right_normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Head back face
    let base = vertices.len() as u32;
    for (pos, uv) in [bl, br, tr, tl].iter().zip(uvs4.iter()) {
        vertices.push(Vertex {
            position: *pos,
            tex_coord: *uv,
            normal: [0.0, 0.0, 1.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

    // Head top triangle
    let base = vertices.len() as u32;
    let uvs3 = [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]];
    for (pos, uv) in [tl, tr, ft].iter().zip(uvs3.iter()) {
        vertices.push(Vertex {
            position: *pos,
            tex_coord: *uv,
            normal: [0.0, 1.0, 0.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2]);

    // Head bottom triangle
    let base = vertices.len() as u32;
    for (pos, uv) in [bl, fb, br].iter().zip(uvs3.iter()) {
        vertices.push(Vertex {
            position: *pos,
            tex_coord: *uv,
            normal: [0.0, -1.0, 0.0],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2]);

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
        tombstone_vertices: &[Vertex],
        tombstone_indices: &[u32],
        tombstone_texture: Option<&TextureData>,
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

        let (player_vertices, player_indices) = generate_player_box();

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

        let tombstone_vertex_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Tombstone Vertex Buffer"),
                contents: bytemuck::cast_slice(tombstone_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let tombstone_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Tombstone Index Buffer"),
            contents: bytemuck::cast_slice(tombstone_indices),
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

        // Create white texture for player (will be tinted by color)
        let white_pixel: [u8; 4] = [255, 255, 255, 255];
        let (_, _, player_texture_bind_group) = create_texture_with_bind_group(
            device,
            queue,
            &texture_layout,
            &sampler,
            &white_pixel,
            1,
            1,
            "Player White Texture",
        );

        // Create tombstone texture (or white fallback)
        let tombstone_texture_bind_group = if let Some(tex) = tombstone_texture {
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
            // Fallback to white texture
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
