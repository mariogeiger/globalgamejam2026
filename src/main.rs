use std::collections::HashMap;
use std::sync::Arc;
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rand::prelude::*;
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent, DeviceEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId, CursorGrabMode},
};

mod map;
mod glb;
mod player;
mod collision;
mod network_player;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
mod webrtc;

use map::{MapVertex, LoadedMap};
#[cfg(not(target_arch = "wasm32"))]
use glb::load_glb;
use glb::load_glb_from_bytes;
use network_player::{PlayerVertex, PlayerUniform, RemotePlayer, Team, generate_player_box};

// Embed the GLB file for WASM builds
#[cfg(target_arch = "wasm32")]
const EMBEDDED_MAP: &[u8] = include_bytes!("../assets/dust2.glb");
use player::Player;
use collision::PhysicsWorld;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

struct MapRenderData {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    bind_group: wgpu::BindGroup,
}

struct PlayerRenderData {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    render_pipeline: wgpu::RenderPipeline,
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    texture_bind_group_layout: wgpu::BindGroupLayout,
    map_meshes: Vec<MapRenderData>,
    player: Player,
    physics: Option<PhysicsWorld>,
    last_frame_time: Instant,
    cursor_grabbed: bool,
    #[allow(dead_code)]
    last_cursor_pos: Option<(f64, f64)>,
    // Spawn points and map bounds for respawning
    spawn_points: Vec<Vec3>,
    map_bounds: Option<(Vec3, Vec3)>,
    // Multiplayer
    local_team: Option<Team>,
    remote_player: Option<RemotePlayer>,
    player_render: Option<PlayerRenderData>,
}

impl GpuState {
    async fn new(window: Arc<Window>, loaded_map: Option<LoadedMap>) -> Self {
        #[cfg(target_arch = "wasm32")]
        let (width, height) = {
            let web_window = web_sys::window().expect("No window");
            let dpr = web_window.device_pixel_ratio();
            let w = (web_window.inner_width().unwrap().as_f64().unwrap() * dpr) as u32;
            let h = (web_window.inner_height().unwrap().as_f64().unwrap() * dpr) as u32;
            (w.max(1), h.max(1))
        };
        
        #[cfg(not(target_arch = "wasm32"))]
        let (width, height) = {
            let size = window.inner_size();
            (size.width.max(1), size.height.max(1))
        };
        
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("Failed to create device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create depth texture
        let (depth_texture, depth_view) = create_depth_texture(&device, width, height);

        // Load shader
        let shader_source = include_str!("map.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Map Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Camera uniform buffer
        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        };
        let camera_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Uniform Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Camera bind group layout
        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform_buffer.as_entire_binding(),
            }],
        });

        // Texture bind group layout
        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Map Pipeline Layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });

        // Render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Map Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[MapVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // BSP faces can be visible from both sides
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
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

        // Default sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Map Texture Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest, // Crispy pixel textures
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Process map data if available
        let (map_meshes, player, physics, spawn_points, map_bounds) = if let Some(loaded_map) = loaded_map {
            let mut gpu_textures: HashMap<String, (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)> = HashMap::new();
            
            // Create GPU textures
            for (name, tex_data) in &loaded_map.textures {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(name),
                    size: wgpu::Extent3d {
                        width: tex_data.width,
                        height: tex_data.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });

                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &tex_data.rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(tex_data.width * 4),
                        rows_per_image: Some(tex_data.height),
                    },
                    wgpu::Extent3d {
                        width: tex_data.width,
                        height: tex_data.height,
                        depth_or_array_layers: 1,
                    },
                );

                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("{} Bind Group", name)),
                    layout: &texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });

                gpu_textures.insert(name.clone(), (texture, view, bind_group));
            }

            // Create placeholder texture for missing textures
            let placeholder_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Placeholder"),
                size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            
            // Magenta/black checker
            let mut placeholder_data = Vec::with_capacity(16 * 16 * 4);
            for y in 0..16 {
                for x in 0..16 {
                    if ((x / 4) + (y / 4)) % 2 == 0 {
                        placeholder_data.extend_from_slice(&[255, 0, 255, 255]);
                    } else {
                        placeholder_data.extend_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &placeholder_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &placeholder_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16 * 4),
                    rows_per_image: Some(16),
                },
                wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
            );
            let placeholder_view = placeholder_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let placeholder_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Placeholder Bind Group"),
                layout: &texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&placeholder_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

            // Create mesh render data
            let mut map_meshes = Vec::new();
            for mesh in &loaded_map.meshes {
                if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                    continue;
                }

                let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{} Vertex Buffer", mesh.texture_name)),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{} Index Buffer", mesh.texture_name)),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let bind_group = if let Some((_, _, bg)) = gpu_textures.get(&mesh.texture_name) {
                    bg.clone()
                } else {
                    placeholder_bind_group.clone()
                };

                map_meshes.push(MapRenderData {
                    vertex_buffer,
                    index_buffer,
                    index_count: mesh.indices.len() as u32,
                    bind_group,
                });
            }

            let spawn_points = loaded_map.spawn_points;
            let map_bounds = Some((loaded_map.bounds_min, loaded_map.bounds_max));
            
            // Pick a random spawn point
            let spawn_idx = rand::thread_rng().gen_range(0..spawn_points.len());
            let initial_spawn = spawn_points[spawn_idx];
            
            let player = Player::new(initial_spawn);
            
            let physics = PhysicsWorld::new(
                &loaded_map.collision_vertices,
                &loaded_map.collision_indices,
                initial_spawn,
            );

            (map_meshes, player, Some(physics), spawn_points, map_bounds)
        } else {
            // No map loaded - create empty state
            let spawn_points = vec![Vec3::new(0.0, 100.0, 0.0)];
            (Vec::new(), Player::new(spawn_points[0]), None, spawn_points, None)
        };

        // Create player rendering resources
        let player_render = {
            // Player shader
            let player_shader_source = include_str!("player.wgsl");
            let player_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Player Shader"),
                source: wgpu::ShaderSource::Wgsl(player_shader_source.into()),
            });

            // Player uniform bind group layout
            let player_uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Player Uniform Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

            // Player pipeline layout
            let player_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Player Pipeline Layout"),
                bind_group_layouts: &[&camera_bind_group_layout, &player_uniform_layout],
                immediate_size: 0,
            });

            // Player render pipeline
            let player_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Player Render Pipeline"),
                layout: Some(&player_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &player_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[PlayerVertex::desc()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &player_shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
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

            // Generate player box mesh
            let (vertices, indices) = generate_player_box();

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Player Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Player Index Buffer"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });

            // Player uniform buffer
            let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Player Uniform Buffer"),
                contents: bytemuck::cast_slice(&[PlayerUniform {
                    model: glam::Mat4::IDENTITY.to_cols_array_2d(),
                    color: [1.0, 0.0, 0.0, 1.0],
                }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Player Bind Group"),
                layout: &player_uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            Some(PlayerRenderData {
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
                pipeline: player_pipeline,
                uniform_buffer,
                bind_group,
            })
        };

        Self {
            window,
            surface,
            device,
            queue,
            config,
            depth_texture,
            depth_view,
            render_pipeline,
            camera_uniform_buffer,
            camera_bind_group,
            camera_bind_group_layout,
            texture_bind_group_layout,
            map_meshes,
            player,
            physics,
            last_frame_time: Instant::now(),
            cursor_grabbed: false,
            last_cursor_pos: None,
            spawn_points,
            map_bounds,
            local_team: None,
            remote_player: None,
            player_render,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            
            // Recreate depth texture
            let (depth_texture, depth_view) = create_depth_texture(&self.device, new_size.width, new_size.height);
            self.depth_texture = depth_texture;
            self.depth_view = depth_view;
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;

        // Cap dt to prevent huge jumps
        let dt = dt.min(0.1);

        // Update player
        self.player.update(dt);

        // Apply physics collision
        if let Some(physics) = &mut self.physics {
            let desired_pos = self.player.position;
            let velocity_y = self.player.velocity.y;
            let (new_pos, on_ground, hit_ceiling) = physics.move_player(desired_pos, velocity_y);
            self.player.position = new_pos;
            self.player.set_on_ground(on_ground, None);
            if hit_ceiling {
                self.player.velocity.y = 0.0; // Stop upward movement on ceiling hit
            }
        }

        // Check if player is outside map bounds and respawn
        if let Some((bounds_min, bounds_max)) = self.map_bounds {
            const RESPAWN_MARGIN: f32 = 500.0; // Distance outside bounds before respawn
            let pos = self.player.position;
            let outside = pos.x < bounds_min.x - RESPAWN_MARGIN
                || pos.x > bounds_max.x + RESPAWN_MARGIN
                || pos.y < bounds_min.y - RESPAWN_MARGIN
                || pos.y > bounds_max.y + RESPAWN_MARGIN
                || pos.z < bounds_min.z - RESPAWN_MARGIN
                || pos.z > bounds_max.z + RESPAWN_MARGIN;
            
            if outside && !self.spawn_points.is_empty() {
                log::info!("Player fell out of map, respawning");
                let spawn_idx = rand::thread_rng().gen_range(0..self.spawn_points.len());
                self.player.respawn(self.spawn_points[spawn_idx]);
            }
        }

        // Send player state to remote peer (WASM only)
        #[cfg(target_arch = "wasm32")]
        if self.local_team.is_some() {
            webrtc::send_player_state_to_peer(self.player.position, self.player.yaw);
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.update();

        // Update camera uniform
        let aspect = self.config.width as f32 / self.config.height as f32;
        let projection = Mat4::perspective_rh(90.0_f32.to_radians(), aspect, 1.0, 10000.0);
        let view = self.player.view_matrix();
        let view_proj = projection * view;

        self.queue.write_buffer(
            &self.camera_uniform_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform { view_proj: view_proj.to_cols_array_2d() }]),
        );

        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Map Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.5,
                            g: 0.7,
                            b: 0.9,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);

            for mesh in &self.map_meshes {
                render_pass.set_bind_group(1, &mesh.bind_group, &[]);
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }

            // Render remote player if connected
            if let (Some(remote), Some(player_render)) = (&self.remote_player, &self.player_render) {
                // Update player uniform with remote player's transform and team color
                let uniform = PlayerUniform {
                    model: remote.model_matrix().to_cols_array_2d(),
                    color: remote.team.color(),
                };
                self.queue.write_buffer(
                    &player_render.uniform_buffer,
                    0,
                    bytemuck::cast_slice(&[uniform]),
                );

                render_pass.set_pipeline(&player_render.pipeline);
                render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
                render_pass.set_bind_group(1, &player_render.bind_group, &[]);
                render_pass.set_vertex_buffer(0, player_render.vertex_buffer.slice(..));
                render_pass.set_index_buffer(player_render.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..player_render.index_count, 0, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    fn grab_cursor(&mut self, grab: bool) {
        self.cursor_grabbed = grab;
        if grab {
            let _ = self.window.set_cursor_grab(CursorGrabMode::Confined)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Locked));
            self.window.set_cursor_visible(false);
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(true);
        }
    }
}

fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

struct App {
    gpu_state: Option<GpuState>,
    loaded_map: Option<LoadedMap>,
}

impl App {
    fn new() -> Self {
        // Try to load map on native platforms
        #[cfg(not(target_arch = "wasm32"))]
        let loaded_map = {
            let glb_path = Path::new("assets/dust2.glb");
            
            match load_glb(glb_path) {
                Ok(map) => {
                    log::info!("Loaded GLB with {} meshes, {} textures", 
                        map.meshes.len(), map.textures.len());
                    log::info!("Spawn point: {:?}", map.spawn_point);
                    log::info!("Collision: {} vertices, {} triangles",
                        map.collision_vertices.len(), map.collision_indices.len());
                    Some(map)
                }
                Err(e) => {
                    log::warn!("Failed to load map: {}. Running without map.", e);
                    log::info!("Place dust2.glb in the assets/ folder.");
                    None
                }
            }
        };
        
        #[cfg(target_arch = "wasm32")]
        let loaded_map = {
            match load_glb_from_bytes(EMBEDDED_MAP) {
                Ok(map) => {
                    log::info!("Loaded embedded GLB with {} meshes, {} textures", 
                        map.meshes.len(), map.textures.len());
                    Some(map)
                }
                Err(e) => {
                    log::warn!("Failed to load embedded map: {}", e);
                    None
                }
            }
        };
        
        Self {
            gpu_state: None,
            loaded_map,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu_state.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title("CS 1.6 Map Viewer");
        
        let window = Arc::new(event_loop.create_window(window_attrs).unwrap());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let loaded_map = self.loaded_map.take();
            self.gpu_state = Some(pollster::block_on(GpuState::new(window.clone(), loaded_map)));
            
            // Grab cursor for FPS controls
            if let Some(state) = &mut self.gpu_state {
                state.grab_cursor(true);
            }
            
            window.request_redraw();
        }

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            
            let canvas = window.canvas().expect("Couldn't get canvas");
            
            let web_window = web_sys::window().expect("No window");
            let dpr = web_window.device_pixel_ratio();
            let width = (web_window.inner_width().unwrap().as_f64().unwrap() * dpr) as u32;
            let height = (web_window.inner_height().unwrap().as_f64().unwrap() * dpr) as u32;
            
            canvas.set_width(width);
            canvas.set_height(height);
            canvas.style().set_css_text("width: 100%; height: 100%; display: block;");
            
            web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| {
                    let dst = doc.get_element_by_id("wasm-container")?;
                    dst.append_child(&canvas).ok()?;
                    Some(())
                })
                .expect("Couldn't append canvas to document body.");

            // Take the loaded map for the async block
            let loaded_map = self.loaded_map.take();
            
            let window_clone = window.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let state = GpuState::new(window_clone.clone(), loaded_map).await;
                GPU_STATE.with(|s| {
                    *s.borrow_mut() = Some(state);
                });
                window_clone.request_redraw();
            });
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(state) = &mut self.gpu_state {
                if state.cursor_grabbed {
                    state.player.handle_mouse_move(delta.0 as f32, delta.1 as f32);
                    state.window.request_redraw();
                }
            }
            
            #[cfg(target_arch = "wasm32")]
            GPU_STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    if state.cursor_grabbed {
                        state.player.handle_mouse_move(delta.0 as f32, delta.1 as f32);
                        state.window.request_redraw();
                    }
                }
            });
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        #[cfg(not(target_arch = "wasm32"))]
        let state = &mut self.gpu_state;
        
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(physical_size) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = state {
                    state.resize(physical_size);
                }
                
                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.resize(physical_size);
                    }
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key_code) = event.physical_key {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(state) = state {
                        match event.state {
                            ElementState::Pressed => {
                                if key_code == KeyCode::Escape {
                                    state.grab_cursor(!state.cursor_grabbed);
                                } else {
                                    state.player.handle_key_press(key_code);
                                }
                            }
                            ElementState::Released => {
                                state.player.handle_key_release(key_code);
                            }
                        }
                        state.window.request_redraw();
                    }
                    
                    #[cfg(target_arch = "wasm32")]
                    GPU_STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            match event.state {
                                ElementState::Pressed => {
                                    if key_code == KeyCode::Escape {
                                        // Exit pointer lock
                                        if let Some(window) = web_sys::window() {
                                            if let Some(document) = window.document() {
                                                document.exit_pointer_lock();
                                            }
                                        }
                                        state.cursor_grabbed = false;
                                    } else {
                                        state.player.handle_key_press(key_code);
                                    }
                                }
                                ElementState::Released => {
                                    state.player.handle_key_release(key_code);
                                }
                            }
                            state.window.request_redraw();
                        }
                    });
                }
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Left, .. } => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = state {
                    if button_state == ElementState::Pressed && !state.cursor_grabbed {
                        state.grab_cursor(true);
                    }
                }
                
                #[cfg(target_arch = "wasm32")]
                if button_state == ElementState::Pressed {
                    // Request pointer lock on click
                    if let Some(window) = web_sys::window() {
                        if let Some(document) = window.document() {
                            if let Some(canvas) = document.get_element_by_id("wasm-container") {
                                if let Some(canvas) = canvas.first_element_child() {
                                    let _ = canvas.request_pointer_lock();
                                    GPU_STATE.with(|s| {
                                        if let Some(state) = s.borrow_mut().as_mut() {
                                            state.cursor_grabbed = true;
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = state {
                    match state.render() {
                        Ok(_) => {
                            // Request continuous redraw for smooth movement
                            state.window.request_redraw();
                        }
                        Err(wgpu::SurfaceError::Lost) => {
                            let size = winit::dpi::PhysicalSize::new(
                                state.config.width,
                                state.config.height,
                            );
                            state.resize(size);
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                        Err(e) => log::error!("Render error: {:?}", e),
                    }
                }

                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        match state.render() {
                            Ok(_) => {
                                // Request continuous redraw for game loop
                                state.window.request_redraw();
                            }
                            Err(wgpu::SurfaceError::Lost) => {
                                let size = winit::dpi::PhysicalSize::new(
                                    state.config.width,
                                    state.config.height,
                                );
                                state.resize(size);
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                            Err(e) => log::error!("Render error: {:?}", e),
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static GPU_STATE: RefCell<Option<GpuState>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&info.to_string().into());
        }));
        console_log::init_with_level(log::Level::Info).expect("Couldn't initialize logger");
        
        // Initialize WebRTC for multiplayer
        webrtc::init_webrtc_client();
        
        // Set up callback for receiving remote player state
        webrtc::set_player_state_callback(|position, yaw| {
            GPU_STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    if let Some(ref mut remote) = state.remote_player {
                        remote.position = position;
                        remote.yaw = yaw;
                    }
                }
            });
        });
        
        // Set up callback for team assignment
        webrtc::set_team_assign_callback(|team| {
            log::info!("Assigned to team: {:?}", team);
            GPU_STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    state.local_team = Some(team);
                    // Create remote player with opposite team
                    let remote_team = match team {
                        Team::A => Team::B,
                        Team::B => Team::A,
                    };
                    state.remote_player = Some(RemotePlayer::new(remote_team));
                }
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init();
    }

    let event_loop = EventLoop::new().unwrap();
    let mut app = App::new();
    
    #[allow(clippy::let_underscore_future)]
    let _ = event_loop.run_app(&mut app);
}

fn main() {
    run();
}
