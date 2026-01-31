use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use web_time::Instant;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rand::prelude::*;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent, DeviceEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

mod config;
mod gpu;
mod map;
mod glb;
mod player;
mod collision;
mod webrtc;

use config::RESPAWN_MARGIN;
use gpu::{create_depth_texture, create_texture_with_bind_group, create_placeholder_bind_group,
         create_vertex_buffer, create_index_buffer, create_uniform_buffer,
         texture_bind_group_layout, camera_bind_group_layout, uniform_bind_group_layout};
use map::{MapVertex, LoadedMap};
use glb::load_glb_from_bytes;
use player::{Player, PlayerVertex, PlayerUniform, RemotePlayer, Team, generate_player_box};
use collision::PhysicsWorld;

const EMBEDDED_MAP: &[u8] = include_bytes!("../assets/dust2.glb");

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
    depth_view: wgpu::TextureView,
    render_pipeline: wgpu::RenderPipeline,
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    map_meshes: Vec<MapRenderData>,
    player: Player,
    physics: Option<PhysicsWorld>,
    last_frame_time: Instant,
    cursor_grabbed: bool,
    spawn_points: Vec<Vec3>,
    map_bounds: Option<(Vec3, Vec3)>,
    local_team: Option<Team>,
    remote_player: Option<RemotePlayer>,
    player_render: Option<PlayerRenderData>,
}

impl GpuState {
    async fn new(window: Arc<Window>, loaded_map: Option<LoadedMap>) -> Self {
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

        let (_, depth_view) = create_depth_texture(&device, width, height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Map Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });

        let camera_uniform = CameraUniform { view_proj: Mat4::IDENTITY.to_cols_array_2d() };
        let camera_uniform_buffer = create_uniform_buffer(&device, &camera_uniform, "Camera Uniform");

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

        let (map_meshes, player, physics, spawn_points, map_bounds) = if let Some(loaded_map) = loaded_map {
            let mut gpu_textures: HashMap<String, wgpu::BindGroup> = HashMap::new();

            for (name, tex_data) in &loaded_map.textures {
                let (_, _, bg) = create_texture_with_bind_group(
                    &device, &queue, &texture_layout, &sampler,
                    &tex_data.rgba, tex_data.width, tex_data.height, name,
                );
                gpu_textures.insert(name.clone(), bg);
            }

            let placeholder = create_placeholder_bind_group(&device, &queue, &texture_layout, &sampler);

            let map_meshes: Vec<_> = loaded_map.meshes.iter()
                .filter(|m| !m.vertices.is_empty() && !m.indices.is_empty())
                .map(|mesh| MapRenderData {
                    vertex_buffer: create_vertex_buffer(&device, &mesh.vertices, &mesh.texture_name),
                    index_buffer: create_index_buffer(&device, &mesh.indices, &mesh.texture_name),
                    index_count: mesh.indices.len() as u32,
                    bind_group: gpu_textures.get(&mesh.texture_name).cloned().unwrap_or_else(|| placeholder.clone()),
                })
                .collect();

            let spawn_idx = rand::thread_rng().gen_range(0..loaded_map.spawn_points.len());
            let initial_spawn = loaded_map.spawn_points[spawn_idx];
            let player = Player::new(initial_spawn);
            let physics = PhysicsWorld::new(&loaded_map.collision_vertices, &loaded_map.collision_indices);

            (map_meshes, player, physics, loaded_map.spawn_points, Some((loaded_map.bounds_min, loaded_map.bounds_max)))
        } else {
            let spawn = vec![Vec3::new(0.0, 100.0, 0.0)];
            (Vec::new(), Player::new(spawn[0]), None, spawn, None)
        };

        let player_render = {
            let player_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Player Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("player.wgsl").into()),
            });

            let player_uniform_layout = uniform_bind_group_layout(&device, "Player Uniform Layout");
            let player_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Player Pipeline Layout"),
                bind_group_layouts: &[&camera_layout, &player_uniform_layout],
                immediate_size: 0,
            });

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
                    cull_mode: Some(wgpu::Face::Back),
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

            let (vertices, indices) = generate_player_box();
            let uniform = PlayerUniform { model: Mat4::IDENTITY.to_cols_array_2d(), color: [1.0, 0.0, 0.0, 1.0] };

            Some(PlayerRenderData {
                vertex_buffer: create_vertex_buffer(&device, &vertices, "Player Vertex"),
                index_buffer: create_index_buffer(&device, &indices, "Player Index"),
                index_count: indices.len() as u32,
                pipeline: player_pipeline,
                uniform_buffer: create_uniform_buffer(&device, &uniform, "Player Uniform"),
                bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Player Bind Group"),
                    layout: &player_uniform_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: create_uniform_buffer(&device, &uniform, "Player Uniform").as_entire_binding(),
                    }],
                }),
            })
        };

        Self {
            window, surface, device, queue, config, depth_view, render_pipeline,
            camera_uniform_buffer, camera_bind_group, map_meshes, player, physics,
            last_frame_time: Instant::now(), cursor_grabbed: false, spawn_points,
            map_bounds, local_team: None, remote_player: None, player_render,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            let (_, depth_view) = create_depth_texture(&self.device, new_size.width, new_size.height);
            self.depth_view = depth_view;
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame_time).as_secs_f32().min(0.1);
        self.last_frame_time = now;

        self.player.update(dt);

        if let Some(ref physics) = self.physics {
            let (new_pos, on_ground, hit_ceiling) = physics.move_player(self.player.position, self.player.velocity.y);
            self.player.position = new_pos;
            self.player.set_on_ground(on_ground, None);
            if hit_ceiling { self.player.velocity.y = 0.0; }
        }

        if let Some((bounds_min, bounds_max)) = self.map_bounds {
            let pos = self.player.position;
            let outside = pos.x < bounds_min.x - RESPAWN_MARGIN || pos.x > bounds_max.x + RESPAWN_MARGIN
                || pos.y < bounds_min.y - RESPAWN_MARGIN || pos.y > bounds_max.y + RESPAWN_MARGIN
                || pos.z < bounds_min.z - RESPAWN_MARGIN || pos.z > bounds_max.z + RESPAWN_MARGIN;

            if outside && !self.spawn_points.is_empty() {
                log::info!("Player fell out of map, respawning");
                let idx = rand::thread_rng().gen_range(0..self.spawn_points.len());
                self.player.respawn(self.spawn_points[idx]);
            }
        }

        if self.local_team.is_some() {
            webrtc::send_player_state_to_peer(self.player.position, self.player.yaw);
        }

        update_coordinates_display(self.player.position);
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.update();

        let aspect = self.config.width as f32 / self.config.height as f32;
        let view_proj = Mat4::perspective_rh(90.0_f32.to_radians(), aspect, 1.0, 10000.0) * self.player.view_matrix();

        self.queue.write_buffer(&self.camera_uniform_buffer, 0,
            bytemuck::cast_slice(&[CameraUniform { view_proj: view_proj.to_cols_array_2d() }]));

        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.5, g: 0.7, b: 0.9, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
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

            if let (Some(remote), Some(pr)) = (&self.remote_player, &self.player_render) {
                let uniform = PlayerUniform { model: remote.model_matrix().to_cols_array_2d(), color: remote.team.color() };
                self.queue.write_buffer(&pr.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

                pass.set_pipeline(&pr.pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_bind_group(1, &pr.bind_group, &[]);
                pass.set_vertex_buffer(0, pr.vertex_buffer.slice(..));
                pass.set_index_buffer(pr.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..pr.index_count, 0, 0..1);
            }
        }

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
        let loaded_map = load_glb_from_bytes(EMBEDDED_MAP).ok();
        if let Some(ref map) = loaded_map {
            log::info!("Loaded GLB: {} meshes, {} textures", map.meshes.len(), map.textures.len());
        }
        Self { gpu_state: None, loaded_map }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu_state.is_some() { return; }

        let window = Arc::new(event_loop.create_window(Window::default_attributes().with_title("CS 1.6 Map Viewer")).unwrap());

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
        canvas.style().set_css_text("width: 100%; height: 100%; display: block;");

        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| doc.get_element_by_id("wasm-container")?.append_child(&canvas).ok())
            .expect("Couldn't append canvas");

        let loaded_map = self.loaded_map.take();
        let window_clone = window.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let state = GpuState::new(window_clone.clone(), loaded_map).await;
            GPU_STATE.with(|s| *s.borrow_mut() = Some(state));
            window_clone.request_redraw();
        });
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                GPU_STATE.with(|s| { if let Some(state) = s.borrow_mut().as_mut() { state.resize(size); } });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    GPU_STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            match event.state {
                                ElementState::Pressed if key == KeyCode::Escape => {
                                    web_sys::window().and_then(|w| w.document()).map(|d| d.exit_pointer_lock());
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
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("wasm-container"))
                    .and_then(|c| c.first_element_child())
                    .map(|canvas| {
                        canvas.request_pointer_lock();
                        GPU_STATE.with(|s| { if let Some(state) = s.borrow_mut().as_mut() { state.cursor_grabbed = true; } });
                    });
            }
            WindowEvent::RedrawRequested => {
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        match state.render() {
                            Ok(_) => state.window.request_redraw(),
                            Err(wgpu::SurfaceError::Lost) => state.resize(winit::dpi::PhysicalSize::new(state.config.width, state.config.height)),
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
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        for (id, val) in [("coord-x", pos.x), ("coord-y", pos.y), ("coord-z", pos.z)] {
            if let Some(e) = doc.get_element_by_id(id) { e.set_text_content(Some(&format!("{:.2}", val))); }
        }
    }
}

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&info.to_string().into())));
    console_log::init_with_level(log::Level::Info).expect("Logger init failed");

    webrtc::init_webrtc_client();

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

    webrtc::set_team_assign_callback(|team| {
        log::info!("Assigned to team: {:?}", team);
        GPU_STATE.with(|s| {
            if let Some(state) = s.borrow_mut().as_mut() {
                state.local_team = Some(team);
                state.remote_player = Some(RemotePlayer::new(match team { Team::A => Team::B, Team::B => Team::A }));
            }
        });
    });

    let event_loop = EventLoop::new().unwrap();
    #[allow(clippy::let_underscore_future)]
    let _ = event_loop.run_app(&mut App::new());
}

fn main() { run(); }
