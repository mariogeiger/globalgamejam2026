use glam::{Mat4, Vec3};
use std::sync::Arc;
use wasm_bindgen::JsCast;
use winit::window::Window;

use crate::assets::{EMBEDDED_MASK, EMBEDDED_PLAYER, EMBEDDED_TOMBSTONE};
use crate::config::PLAYER_HEIGHT;
use crate::game::GameState;
use crate::glb::load_mesh_from_bytes;
use crate::gpu::{camera_bind_group_layout, create_depth_texture};
use crate::mesh::Mesh;

/// Check if WebGPU is available in this browser
pub fn check_webgpu_support() -> bool {
    if let Some(window) = web_sys::window() {
        let navigator = window.navigator();
        // Check if navigator.gpu exists
        let gpu = js_sys::Reflect::get(&navigator, &"gpu".into()).ok();
        if let Some(gpu_val) = gpu
            && !gpu_val.is_undefined()
            && !gpu_val.is_null()
        {
            return true;
        }
    }
    false
}

/// Show the WebGPU error overlay
pub fn show_webgpu_error() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        // Hide main menu
        if let Some(menu) = doc.get_element_by_id("main-menu") {
            let _ = menu
                .dyn_ref::<web_sys::HtmlElement>()
                .map(|e| e.style().set_property("display", "none").ok());
        }
        // Show error overlay
        if let Some(overlay) = doc.get_element_by_id("webgpu-error") {
            let _ = overlay
                .dyn_ref::<web_sys::HtmlElement>()
                .map(|e| e.style().set_property("display", "flex").ok());
        }
    }
}

pub mod camera;
pub mod cone;
pub mod hud;
pub mod map;
pub mod player;
pub mod postprocess;
pub mod threat;
pub mod traits;
pub mod view_mask;

use camera::CameraState;
use cone::ConeRenderer;
use hud::HudRenderer;
use map::MapRenderer;
use player::PlayerRenderer;
use postprocess::{PostProcessApplyParams, PostProcessor};
use threat::ThreatIndicatorRenderer;
use traits::Renderable;
use view_mask::{MASK_ANIM_DURATION, ViewMaskRenderer};

use crate::player::MaskType;

pub struct RenderContext {
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}

impl RenderContext {
    pub async fn new(window: Arc<Window>) -> Self {
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

        Self {
            window,
            surface,
            device,
            queue,
            config,
        }
    }
}

pub struct Renderer {
    pub ctx: RenderContext,
    camera: CameraState,
    depth_view: wgpu::TextureView,
    map_renderer: MapRenderer,
    player_renderer: PlayerRenderer,
    cone_renderer: ConeRenderer,
    postprocessor: PostProcessor,
    hud_renderer: HudRenderer,
    view_mask_renderer: ViewMaskRenderer,
    threat_renderer: ThreatIndicatorRenderer,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, map_mesh: &Mesh) -> Self {
        let ctx = RenderContext::new(window).await;

        let camera_layout = camera_bind_group_layout(&ctx.device);

        let camera = CameraState::new(&ctx.device);
        let (_, depth_view) =
            create_depth_texture(&ctx.device, ctx.config.width, ctx.config.height);

        let map_renderer = MapRenderer::new(
            &ctx.device,
            &ctx.queue,
            &camera_layout,
            ctx.config.format,
            map_mesh,
        );

        // Load player model, rotate 180° and scale to PLAYER_HEIGHT
        let mut player_mesh = load_mesh_from_bytes(EMBEDDED_PLAYER).expect("Failed to load player");
        player_mesh.rotate_y_180();
        player_mesh.rescale(PLAYER_HEIGHT / player_mesh.bounding_box().height());

        // Load tombstone model and scale up 100x
        let mut tombstone_mesh =
            load_mesh_from_bytes(EMBEDDED_TOMBSTONE).expect("Failed to load tombstone");
        tombstone_mesh.rescale(100.0);

        let player_renderer = PlayerRenderer::new(
            &ctx.device,
            &ctx.queue,
            &camera_layout,
            ctx.config.format,
            &player_mesh,
            &tombstone_mesh,
        );

        let cone_renderer = ConeRenderer::new(&ctx.device, &camera_layout, ctx.config.format);

        let postprocessor = PostProcessor::new(
            &ctx.device,
            ctx.config.format,
            ctx.config.width,
            ctx.config.height,
        );

        let hud_renderer = HudRenderer::new(&ctx.device, ctx.config.format);

        // Load mask model; render in view space (relative to camera)
        let mut mask_mesh = load_mesh_from_bytes(EMBEDDED_MASK).expect("Failed to load view mask");
        mask_mesh.rotate_y_180();
        mask_mesh.rescale(20.0);
        mask_mesh.translate(0.0, -3.0, 0.0);
        let view_mask_renderer = ViewMaskRenderer::new(&ctx.device, ctx.config.format, &mask_mesh);

        let threat_renderer = ThreatIndicatorRenderer::new(&ctx.device, ctx.config.format);

        Self {
            ctx,
            camera,
            depth_view,
            map_renderer,
            player_renderer,
            cone_renderer,
            postprocessor,
            hud_renderer,
            view_mask_renderer,
            threat_renderer,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.ctx.config.width = width;
            self.ctx.config.height = height;
            self.ctx
                .surface
                .configure(&self.ctx.device, &self.ctx.config);

            let (_, depth_view) = create_depth_texture(&self.ctx.device, width, height);
            self.depth_view = depth_view;

            self.postprocessor
                .resize(&self.ctx.device, width, height, self.ctx.config.format);
        }
    }

    pub fn render_frame(&mut self, game: &GameState) -> Result<(), wgpu::SurfaceError> {
        let aspect = self.ctx.config.width as f32 / self.ctx.config.height as f32;
        let projection = Mat4::perspective_rh(90.0_f32.to_radians(), aspect, 1.0, 10000.0);
        let view = game.player.view_matrix();
        let view_proj = projection * view;

        self.camera
            .update(&self.ctx.queue, view_proj, view, game.player.velocity);

        let output = self.ctx.surface.get_current_texture()?;
        let swapchain_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scene Pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: self.postprocessor.offscreen_view(),
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
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: self.postprocessor.position_view(),
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: self.postprocessor.velocity_view(),
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
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

            self.map_renderer.render(&mut pass, &self.camera.bind_group);

            // Debug: log remote player info
            for (id, remote) in &game.remote_players {
                log::debug!(
                    "Render: remote {} at [{:.1}, {:.1}, {:.1}], alive={}, vel=[{:.1}, {:.1}, {:.1}]",
                    id,
                    remote.position.x,
                    remote.position.y,
                    remote.position.z,
                    remote.is_alive,
                    remote.velocity.x,
                    remote.velocity.y,
                    remote.velocity.z,
                );
            }

            let alive_players: Vec<_> = game
                .remote_players
                .values()
                .filter(|remote| remote.is_alive)
                .map(|remote| {
                    (
                        remote.model_matrix(),
                        [1.0, 1.0, 1.0, 1.0_f32],
                        remote.velocity,
                    )
                })
                .collect();

            log::debug!(
                "Rendering {} alive players, {} death markers",
                alive_players.len(),
                game.death_locations.len()
            );

            let dead_players: Vec<_> = game
                .death_locations
                .iter()
                .map(|marker| (marker.model_matrix(), [1.0, 1.0, 1.0, 1.0_f32], Vec3::ZERO))
                .collect();

            self.player_renderer.render(
                &mut pass,
                &self.ctx.queue,
                &self.ctx.device,
                &self.camera.bind_group,
                &alive_players,
                &dead_players,
            );

            // Render vision cones for Hunter mask players (transparent, after opaque geometry)
            let hunter_cones: Vec<_> = game
                .remote_players
                .values()
                .filter(|remote| remote.is_alive && remote.mask == MaskType::Hunter)
                .map(|remote| (remote.position, remote.yaw, remote.pitch))
                .collect();

            self.cone_renderer.render(
                &mut pass,
                &self.ctx.queue,
                &self.ctx.device,
                &self.camera.bind_group,
                &hunter_cones,
            );

            // Mask animation on mask change (plays once)
            if let Some(change_time) = game.mask_change_time {
                let elapsed = game.time - change_time;
                if elapsed < MASK_ANIM_DURATION {
                    let progress = elapsed / MASK_ANIM_DURATION;
                    self.view_mask_renderer.render(
                        &mut pass,
                        &self.ctx.queue,
                        projection,
                        progress,
                        game.player.mask as u8,
                    );
                }
            }
        }

        self.postprocessor.apply(
            &mut encoder,
            &self.ctx.queue,
            &swapchain_view,
            PostProcessApplyParams {
                width: self.ctx.config.width,
                height: self.ctx.config.height,
                mask_type: game.player.mask as u8,
                time: game.time,
            },
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &swapchain_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            if game.player.mask != MaskType::Coward {
                let (targeting_progress, has_target) = game.get_targeting_info();
                self.hud_renderer.render(
                    &mut pass,
                    &self.ctx.queue,
                    projection,
                    targeting_progress,
                    has_target,
                );
            }

            // Render threat indicators for enemies looking at us
            let threats = game.get_threats();
            if !threats.is_empty() {
                let view = game.player.view_matrix();
                let threat_angles: Vec<f32> = threats
                    .iter()
                    .map(|(_, pos)| self.calculate_threat_angle(*pos, view, projection))
                    .collect();
                self.threat_renderer.render(
                    &mut pass,
                    &self.ctx.queue,
                    self.ctx.config.width as f32,
                    self.ctx.config.height as f32,
                    &threat_angles,
                    game.time,
                );
            }
        }

        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }

    pub fn request_redraw(&self) {
        self.ctx.window.request_redraw();
    }

    pub fn width(&self) -> u32 {
        self.ctx.config.width
    }

    pub fn height(&self) -> u32 {
        self.ctx.config.height
    }

    /// Calculate screen-space angle for a threat indicator
    /// Returns angle in radians where 0 = right, PI/2 = up
    fn calculate_threat_angle(&self, world_pos: Vec3, view: Mat4, projection: Mat4) -> f32 {
        // Transform world position to view space first
        let view_pos = view * world_pos.extend(1.0);

        // Transform to clip space
        let view_proj = projection * view;
        let clip_pos = view_proj * world_pos.extend(1.0);

        // Check if behind camera
        if clip_pos.w <= 0.0 {
            // Behind camera - use view space position to calculate angle
            // In view space: -Z is forward, X is right, Y is up
            return view_pos.y.atan2(view_pos.x);
        }

        // Perspective divide to get NDC
        let ndc = clip_pos.truncate() / clip_pos.w;

        // Calculate angle from center of screen
        ndc.y.atan2(ndc.x)
    }
}
