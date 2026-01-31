use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rand::Rng;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

mod collision;
mod config;
mod glb;
mod gpu;
mod map;
mod player;
mod webrtc;

use collision::PhysicsWorld;
use config::{
    EYE_HEIGHT, PLAYER_HEIGHT, RESPAWN_DELAY, RESPAWN_MARGIN, TARGETING_ANGLE, TARGETING_DURATION,
};
use glb::load_glb_from_bytes;
use gpu::{
    camera_bind_group_layout, create_depth_texture, create_index_buffer,
    create_placeholder_bind_group, create_render_target_texture, create_texture_with_bind_group,
    create_uniform_buffer, create_vertex_buffer, depth_texture_bind_group_layout,
    texture_bind_group_layout, uniform_bind_group_layout,
};
use map::{LoadedMap, MapVertex};
use player::{Player, PlayerRenderer, RemotePlayer, Team};

const EMBEDDED_MAP: &[u8] = include_bytes!("../assets/dust2.glb");

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
}

/// Post-processing: motion blur, smear, screen/depth, and camera clip planes.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct PostProcessUniform {
    blur_direction: [f32; 2],
    blur_strength: f32,
    /// How much of the previous frame to blend (0 = no smear, ~0.9 = strong trails).
    smear_factor: f32,
    /// Viewport size (width, height) for pixel (x,y) from tex_coord.
    resolution: [f32; 2],
    /// Near and far clip plane distances (for linear depth from depth buffer).
    depth_near: f32,
    depth_far: f32,
}

/// Fullscreen quad for post-processing pass (position + uv).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct PostProcessVertex {
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

struct MapRenderData {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    bind_group: wgpu::BindGroup,
}

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    render_pipeline: wgpu::RenderPipeline,
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    map_meshes: Vec<MapRenderData>,
    player: Player,
    physics: PhysicsWorld,
    last_frame_time: Instant,
    cursor_grabbed: bool,
    spawn_points: Vec<Vec3>,
    map_bounds: (Vec3, Vec3),
    local_team: Option<Team>,
    remote_players: HashMap<u64, RemotePlayer>,
    player_renderer: PlayerRenderer,
    // Post-processing: scene render target and fullscreen pass
    offscreen_texture: wgpu::Texture,
    offscreen_view: wgpu::TextureView,
    postprocess_pipeline: wgpu::RenderPipeline,
    postprocess_bind_group: wgpu::BindGroup,
    postprocess_depth_bind_group: wgpu::BindGroup,
    postprocess_depth_sampler: wgpu::Sampler,
    postprocess_uniform_buffer: wgpu::Buffer,
    postprocess_uniform_bind_group: wgpu::BindGroup,
    postprocess_previous_bind_group_0: wgpu::BindGroup,
    postprocess_previous_bind_group_1: wgpu::BindGroup,
    postprocess_sampler: wgpu::Sampler,
    fullscreen_quad_buffer: wgpu::Buffer,
    // History buffers for previous-frame smearing (ping-pong)
    history_texture_0: wgpu::Texture,
    history_view_0: wgpu::TextureView,
    history_texture_1: wgpu::Texture,
    history_view_1: wgpu::TextureView,
    history_index: u8,
    present_pipeline: wgpu::RenderPipeline,
    present_bind_group_0: wgpu::BindGroup,
    present_bind_group_1: wgpu::BindGroup,
    first_frame: bool,
}

impl GpuState {
    async fn new(window: Arc<Window>, loaded_map: LoadedMap) -> Self {
        let (width, height) = {
            let web_window = web_sys::window().expect("No window");
            let dpr = web_window.device_pixel_ratio();
            let w = (web_window.inner_width().unwrap().as_f64().unwrap() * dpr) as u32;
            let h = (web_window.inner_height().unwrap().as_f64().unwrap() * dpr) as u32;
            (w.max(1), h.max(1))
        };

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
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
        let surface_format = surface_caps
            .formats
            .iter()
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

        let (_, depth_view) = create_depth_texture(&device, width, height);

        let (offscreen_texture, offscreen_view) =
            create_render_target_texture(&device, width, height, config.format);

        let (history_texture_0, history_view_0) =
            create_render_target_texture(&device, width, height, config.format);
        let (history_texture_1, history_view_1) =
            create_render_target_texture(&device, width, height, config.format);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Map Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });

        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        };
        let camera_uniform_buffer =
            create_uniform_buffer(&device, &camera_uniform, "Camera Uniform");

        let camera_layout = camera_bind_group_layout(&device);
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform_buffer.as_entire_binding(),
            }],
        });

        let texture_layout = texture_bind_group_layout(&device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Map Pipeline Layout"),
            bind_group_layouts: &[&camera_layout, &texture_layout],
            immediate_size: 0,
        });

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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Map Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Post-processing: fullscreen pass over scene texture
        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Postprocess Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("postprocess.wgsl").into()),
        });
        let postprocess_texture_layout = texture_bind_group_layout(&device);
        let postprocess_depth_layout = depth_texture_bind_group_layout(&device);
        let postprocess_uniform_layout =
            uniform_bind_group_layout(&device, "Postprocess Uniform Layout");
        let postprocess_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Postprocess Pipeline Layout"),
                bind_group_layouts: &[
                    &postprocess_texture_layout,
                    &postprocess_uniform_layout,
                    &postprocess_texture_layout,
                    &postprocess_depth_layout,
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
                    format: config.format,
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
        let postprocess_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Postprocess Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let postprocess_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Bind Group (scene)"),
            layout: &postprocess_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&offscreen_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&postprocess_sampler),
                },
            ],
        });
        let postprocess_depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Postprocess Depth Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let postprocess_depth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Depth Bind Group"),
            layout: &postprocess_depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&postprocess_depth_sampler),
                },
            ],
        });
        let postprocess_previous_bind_group_0 =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 0"),
                layout: &postprocess_texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&history_view_0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&postprocess_sampler),
                    },
                ],
            });
        let postprocess_previous_bind_group_1 =
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Postprocess Previous Bind Group 1"),
                layout: &postprocess_texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&history_view_1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&postprocess_sampler),
                    },
                ],
            });
        let postprocess_uniform = PostProcessUniform {
            blur_direction: [0.0, 0.0],
            blur_strength: 0.0,
            smear_factor: 0.85,
            resolution: [width as f32, height as f32],
            depth_near: 1.0,
            depth_far: 10000.0,
        };
        let postprocess_uniform_buffer =
            create_uniform_buffer(&device, &postprocess_uniform, "Postprocess Uniform");
        let postprocess_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Postprocess Uniform Bind Group"),
            layout: &postprocess_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: postprocess_uniform_buffer.as_entire_binding(),
            }],
        });
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
        let fullscreen_quad_buffer =
            create_vertex_buffer(&device, &FULLSCREEN_QUAD, "Fullscreen Quad");

        // Present pass: blit history texture to swapchain
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Present Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });
        let present_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Present Pipeline Layout"),
                bind_group_layouts: &[&postprocess_texture_layout],
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
                    format: config.format,
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
        let present_bind_group_0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Present Bind Group 0"),
            layout: &postprocess_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&history_view_0),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&postprocess_sampler),
                },
            ],
        });
        let present_bind_group_1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Present Bind Group 1"),
            layout: &postprocess_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&history_view_1),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&postprocess_sampler),
                },
            ],
        });

        let mut gpu_textures: HashMap<String, wgpu::BindGroup> = HashMap::new();

        for (name, tex_data) in &loaded_map.textures {
            let (_, _, bg) = create_texture_with_bind_group(
                &device,
                &queue,
                &texture_layout,
                &sampler,
                &tex_data.rgba,
                tex_data.width,
                tex_data.height,
                name,
            );
            gpu_textures.insert(name.clone(), bg);
        }

        let placeholder = create_placeholder_bind_group(&device, &queue, &texture_layout, &sampler);

        let map_meshes: Vec<_> = loaded_map
            .meshes
            .iter()
            .filter(|m| !m.vertices.is_empty() && !m.indices.is_empty())
            .map(|mesh| MapRenderData {
                vertex_buffer: create_vertex_buffer(&device, &mesh.vertices, &mesh.texture_name),
                index_buffer: create_index_buffer(&device, &mesh.indices, &mesh.texture_name),
                index_count: mesh.indices.len() as u32,
                bind_group: gpu_textures
                    .get(&mesh.texture_name)
                    .cloned()
                    .unwrap_or_else(|| placeholder.clone()),
            })
            .collect();

        let spawn_idx = rand::rng().random_range(0..loaded_map.spawn_points.len());
        let initial_spawn = loaded_map.spawn_points[spawn_idx];
        let player = Player::new(initial_spawn);
        let physics = PhysicsWorld::new(
            &loaded_map.collision_vertices,
            &loaded_map.collision_indices,
        )
        .expect("Failed to create physics world");
        let spawn_points = loaded_map.spawn_points;
        let map_bounds = (loaded_map.bounds_min, loaded_map.bounds_max);

        let player_renderer = PlayerRenderer::new(&device, &camera_layout, config.format);

        // Create debug mannequins for testing (one per team at their spawn points)
        let mut remote_players = HashMap::new();
        let mannequin_a = RemotePlayer::new(Team::A);
        let mannequin_b = RemotePlayer::new(Team::B);
        log::info!(
            "Creating mannequins - A at {:?}, B at {:?}, player at {:?}",
            mannequin_a.position,
            mannequin_b.position,
            initial_spawn
        );
        remote_players.insert(u64::MAX, mannequin_a);
        remote_players.insert(u64::MAX - 1, mannequin_b);

        Self {
            window,
            surface,
            device,
            queue,
            config,
            depth_view,
            render_pipeline,
            camera_uniform_buffer,
            camera_bind_group,
            map_meshes,
            player,
            physics,
            last_frame_time: Instant::now(),
            cursor_grabbed: false,
            spawn_points,
            map_bounds,
            local_team: None,
            remote_players,
            player_renderer,
            offscreen_texture,
            offscreen_view,
            postprocess_pipeline,
            postprocess_bind_group,
            postprocess_depth_bind_group,
            postprocess_depth_sampler,
            postprocess_uniform_buffer,
            postprocess_uniform_bind_group,
            postprocess_previous_bind_group_0,
            postprocess_previous_bind_group_1,
            postprocess_sampler,
            fullscreen_quad_buffer,
            history_texture_0,
            history_view_0,
            history_texture_1,
            history_view_1,
            history_index: 0,
            present_pipeline,
            present_bind_group_0,
            present_bind_group_1,
            first_frame: true,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            let (_, depth_view) =
                create_depth_texture(&self.device, new_size.width, new_size.height);
            self.depth_view = depth_view;
            let (offscreen_texture, offscreen_view) = create_render_target_texture(
                &self.device,
                new_size.width,
                new_size.height,
                self.config.format,
            );
            self.offscreen_texture = offscreen_texture;
            self.offscreen_view = offscreen_view;

            let (history_texture_0, history_view_0) = create_render_target_texture(
                &self.device,
                new_size.width,
                new_size.height,
                self.config.format,
            );
            let (history_texture_1, history_view_1) = create_render_target_texture(
                &self.device,
                new_size.width,
                new_size.height,
                self.config.format,
            );
            self.history_texture_0 = history_texture_0;
            self.history_view_0 = history_view_0;
            self.history_texture_1 = history_texture_1;
            self.history_view_1 = history_view_1;

            let postprocess_texture_layout = texture_bind_group_layout(&self.device);
            let postprocess_depth_layout = depth_texture_bind_group_layout(&self.device);
            self.postprocess_bind_group =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Postprocess Bind Group (scene)"),
                    layout: &postprocess_texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.offscreen_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.postprocess_sampler),
                        },
                    ],
                });
            self.postprocess_depth_bind_group =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Postprocess Depth Bind Group"),
                    layout: &postprocess_depth_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(
                                &self.postprocess_depth_sampler,
                            ),
                        },
                    ],
                });
            self.postprocess_previous_bind_group_0 =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Postprocess Previous Bind Group 0"),
                    layout: &postprocess_texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.history_view_0),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.postprocess_sampler),
                        },
                    ],
                });
            self.postprocess_previous_bind_group_1 =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Postprocess Previous Bind Group 1"),
                    layout: &postprocess_texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.history_view_1),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.postprocess_sampler),
                        },
                    ],
                });
            self.present_bind_group_0 = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Present Bind Group 0"),
                layout: &postprocess_texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_view_0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.postprocess_sampler),
                    },
                ],
            });
            self.present_bind_group_1 = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Present Bind Group 1"),
                layout: &postprocess_texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.history_view_1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.postprocess_sampler),
                    },
                ],
            });
            self.first_frame = true;
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame_time).as_secs_f32().min(0.1);
        self.last_frame_time = now;

        self.player.update(dt);

        let (new_pos, on_ground) = self
            .physics
            .move_player(self.player.position, self.player.velocity);
        self.player.position = new_pos;
        self.player.set_on_ground(on_ground, None);

        let (bounds_min, bounds_max) = self.map_bounds;
        let pos = self.player.position;
        let outside = pos.x < bounds_min.x - RESPAWN_MARGIN
            || pos.x > bounds_max.x + RESPAWN_MARGIN
            || pos.y < bounds_min.y - RESPAWN_MARGIN
            || pos.y > bounds_max.y + RESPAWN_MARGIN
            || pos.z < bounds_min.z - RESPAWN_MARGIN
            || pos.z > bounds_max.z + RESPAWN_MARGIN;

        if outside {
            log::info!("Player fell out of map, respawning");
            // Use team-specific spawn points if assigned, otherwise use generic
            if let Some(team) = self.local_team {
                let spawns = team.spawn_points();
                if !spawns.is_empty() {
                    let idx = rand::rng().random_range(0..spawns.len());
                    let spawn = spawns[idx];
                    self.player.respawn(Vec3::new(spawn[0], spawn[1], spawn[2]));
                }
            } else if !self.spawn_points.is_empty() {
                let idx = rand::rng().random_range(0..self.spawn_points.len());
                self.player.respawn(self.spawn_points[idx]);
            }
        }

        if self.local_team.is_some() {
            webrtc::send_player_state_to_peers(self.player.position, self.player.yaw);
        }

        // Targeting system: kill enemies that stay in view cone for TARGETING_DURATION
        let eye_pos = self.player.position + Vec3::new(0.0, EYE_HEIGHT, 0.0);
        let look_dir = Vec3::new(
            self.player.yaw.sin() * self.player.pitch.cos(),
            self.player.pitch.sin(),
            -self.player.yaw.cos() * self.player.pitch.cos(),
        )
        .normalize();
        let half_angle_rad = (TARGETING_ANGLE / 2.0).to_radians();

        for remote in self.remote_players.values_mut() {
            // Handle dead players - respawn after delay
            if !remote.is_alive {
                remote.dead_time += dt;
                if remote.dead_time >= RESPAWN_DELAY {
                    remote.respawn();
                    log::info!("Enemy respawned!");
                }
                continue;
            }

            // Skip teammates for targeting
            if let Some(local_team) = self.local_team
                && remote.team == local_team
            {
                remote.targeted_time = 0.0;
                continue;
            }

            // Calculate angle to enemy (aim at center mass)
            let enemy_center = remote.position + Vec3::new(0.0, PLAYER_HEIGHT / 2.0, 0.0);
            let to_enemy = enemy_center - eye_pos;
            let distance = to_enemy.length();

            if distance < 1.0 {
                remote.targeted_time = 0.0;
                continue;
            }

            let to_enemy_normalized = to_enemy / distance;
            let dot = look_dir.dot(to_enemy_normalized).clamp(-1.0, 1.0);
            let angle = dot.acos();

            // Check if enemy is within targeting cone and visible
            if angle < half_angle_rad && self.physics.is_visible(eye_pos, enemy_center) {
                remote.targeted_time += dt;
                if remote.targeted_time >= TARGETING_DURATION {
                    remote.is_alive = false;
                    log::info!("Enemy killed!");
                }
            } else {
                remote.targeted_time = 0.0;
            }
        }

        update_coordinates_display(self.player.position);
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.update();

        let aspect = self.config.width as f32 / self.config.height as f32;
        let view_proj = Mat4::perspective_rh(90.0_f32.to_radians(), aspect, 1.0, 10000.0)
            * self.player.view_matrix();

        self.queue.write_buffer(
            &self.camera_uniform_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform {
                view_proj: view_proj.to_cols_array_2d(),
            }]),
        );

        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Pass 1: render scene to offscreen texture
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.offscreen_view,
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

            pass.set_pipeline(&self.render_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);

            for mesh in &self.map_meshes {
                pass.set_bind_group(1, &mesh.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }

            // Collect player render data
            let players: Vec<_> = self
                .remote_players
                .values()
                .map(|remote| {
                    let color = if remote.is_alive {
                        remote.team.color()
                    } else {
                        [0.1, 0.1, 0.1, 1.0]
                    };
                    (remote.model_matrix(), color)
                })
                .collect();

            self.player_renderer.render(
                &mut pass,
                &self.queue,
                &self.device,
                &self.camera_bind_group,
                &players,
            );
        }

        // Motion blur: direction from horizontal velocity, strength from speed (UV-space scale)
        let vel_xz = glam::Vec2::new(self.player.velocity.x, self.player.velocity.z);
        let speed = vel_xz.length();
        let (blur_direction, blur_strength) = if speed > 1.0 {
            let dir = vel_xz.normalize();
            // Scale so ~300 units/s gives ~0.02 UV offset
            let strength = (speed / 300.0) * 0.02f32;
            ([dir.x, dir.y], strength)
        } else {
            ([0.0f32, 0.0], 0.0)
        };
        let smear_factor = if self.first_frame { 0.0 } else { 0.85 };
        self.first_frame = false;

        self.queue.write_buffer(
            &self.postprocess_uniform_buffer,
            0,
            bytemuck::cast_slice(&[PostProcessUniform {
                blur_direction,
                blur_strength,
                smear_factor,
                resolution: [self.config.width as f32, self.config.height as f32],
                depth_near: 1.0,
                depth_far: 10000.0,
            }]),
        );

        let prev_idx = self.history_index as usize;
        let next_idx = 1 - prev_idx;
        let history_next_view = if next_idx == 0 {
            &self.history_view_0
        } else {
            &self.history_view_1
        };
        let postprocess_previous_bind_group = if prev_idx == 0 {
            &self.postprocess_previous_bind_group_0
        } else {
            &self.postprocess_previous_bind_group_1
        };
        let present_bind_group = if next_idx == 0 {
            &self.present_bind_group_0
        } else {
            &self.present_bind_group_1
        };

        // Pass 2: post-process (scene + previous) → history buffer
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Postprocess Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: history_next_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
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

        // Pass 3: present history → swapchain
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Present Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
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

        self.history_index = next_idx as u8;

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

struct App {
    gpu_state: Option<GpuState>,
    loaded_map: Option<LoadedMap>,
}

impl App {
    fn new() -> Self {
        let loaded_map = load_glb_from_bytes(EMBEDDED_MAP).expect("Failed to load map");
        log::info!(
            "Loaded GLB: {} meshes, {} textures",
            loaded_map.meshes.len(),
            loaded_map.textures.len()
        );
        Self {
            gpu_state: None,
            loaded_map: Some(loaded_map),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu_state.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("CS 1.6 Map Viewer"))
                .unwrap(),
        );

        use winit::platform::web::WindowExtWebSys;
        let canvas = window.canvas().expect("No canvas");

        let web_window = web_sys::window().expect("No window");
        let dpr = web_window.device_pixel_ratio();
        let (w, h) = (
            (web_window.inner_width().unwrap().as_f64().unwrap() * dpr) as u32,
            (web_window.inner_height().unwrap().as_f64().unwrap() * dpr) as u32,
        );
        canvas.set_width(w);
        canvas.set_height(h);
        canvas
            .style()
            .set_css_text("width: 100%; height: 100%; display: block;");

        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| {
                doc.get_element_by_id("wasm-container")?
                    .append_child(&canvas)
                    .ok()
            })
            .expect("Couldn't append canvas");

        let loaded_map = self.loaded_map.take().expect("Map already consumed");
        let window_clone = window.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let state = GpuState::new(window_clone.clone(), loaded_map).await;
            GPU_STATE.with(|s| *s.borrow_mut() = Some(state));
            window_clone.request_redraw();
        });
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        let DeviceEvent::MouseMotion { delta } = event else {
            return;
        };
        GPU_STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            if state.cursor_grabbed {
                state
                    .player
                    .handle_mouse_move(delta.0 as f32, delta.1 as f32);
                state.window.request_redraw();
            }
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.resize(size);
                    }
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    GPU_STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            match event.state {
                                ElementState::Pressed if key == KeyCode::Escape => {
                                    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
                                        d.exit_pointer_lock();
                                    }
                                    state.cursor_grabbed = false;
                                }
                                ElementState::Pressed => state.player.handle_key_press(key),
                                ElementState::Released => state.player.handle_key_release(key),
                            }
                            state.window.request_redraw();
                        }
                    });
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(canvas) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("wasm-container"))
                    .and_then(|c| c.first_element_child())
                {
                    canvas.request_pointer_lock();
                    GPU_STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            state.cursor_grabbed = true;
                        }
                    });
                }
            }
            WindowEvent::RedrawRequested => {
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        match state.render() {
                            Ok(_) => state.window.request_redraw(),
                            Err(wgpu::SurfaceError::Lost) => {
                                state.resize(winit::dpi::PhysicalSize::new(
                                    state.config.width,
                                    state.config.height,
                                ))
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

thread_local! {
    static GPU_STATE: RefCell<Option<GpuState>> = const { RefCell::new(None) };
}

fn update_coordinates_display(pos: Vec3) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(e) = doc.get_element_by_id("local-pos")
    {
        e.set_text_content(Some(&format!("[{:.1}, {:.1}, {:.1}]", pos.x, pos.y, pos.z)));
    }
}

fn update_team_counts_display(
    local_team: Option<Team>,
    remote_players: &HashMap<u64, RemotePlayer>,
) {
    let mut team_a = 0;
    let mut team_b = 0;

    // Count local player
    if let Some(team) = local_team {
        match team {
            Team::A => team_a += 1,
            Team::B => team_b += 1,
        }
    }

    // Count remote players
    for remote in remote_players.values() {
        match remote.team {
            Team::A => team_a += 1,
            Team::B => team_b += 1,
        }
    }

    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(e) = doc.get_element_by_id("team-a-count") {
            e.set_text_content(Some(&team_a.to_string()));
        }
        if let Some(e) = doc.get_element_by_id("team-b-count") {
            e.set_text_content(Some(&team_b.to_string()));
        }
    }
}

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
    console_log::init_with_level(log::Level::Info).expect("Logger init failed");

    webrtc::init_webrtc_client();

    webrtc::set_player_state_callback(|peer_id, position, yaw| {
        GPU_STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            if let Some(remote) = state.remote_players.get_mut(&peer_id) {
                remote.position = position;
                remote.yaw = yaw;
            }
        });
    });

    webrtc::set_team_assign_callback(|team| {
        log::info!("Assigned to team: {team:?}");
        GPU_STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            state.local_team = Some(team);

            // Respawn local player at their team's spawn point
            let spawns = team.spawn_points();
            if !spawns.is_empty() {
                let idx = rand::rng().random_range(0..spawns.len());
                let spawn = spawns[idx];
                state
                    .player
                    .respawn(Vec3::new(spawn[0], spawn[1], spawn[2]));
            }

            update_team_counts_display(state.local_team, &state.remote_players);
        });
    });

    webrtc::set_peer_joined_callback(|peer_id, team| {
        log::info!("Peer {} joined on team {:?}", peer_id, team);
        GPU_STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            let remote = RemotePlayer::new(team);
            state.remote_players.insert(peer_id, remote);
            update_team_counts_display(state.local_team, &state.remote_players);
        });
    });

    webrtc::set_peer_left_callback(|peer_id| {
        log::info!("Peer {} left", peer_id);
        GPU_STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            state.remote_players.remove(&peer_id);
            update_team_counts_display(state.local_team, &state.remote_players);
        });
    });

    let event_loop = EventLoop::new().unwrap();
    #[allow(clippy::let_underscore_future)]
    let _ = event_loop.run_app(&mut App::new());
}

fn main() {
    run();
}
