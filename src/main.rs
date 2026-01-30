use std::sync::Arc;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
mod webrtc;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
}

const VERTICES: &[Vertex] = &[
    // Front face (red)
    Vertex { position: [-0.5, -0.5,  0.5], color: [1.0, 0.4, 0.4] },
    Vertex { position: [ 0.5, -0.5,  0.5], color: [1.0, 0.4, 0.4] },
    Vertex { position: [ 0.5,  0.5,  0.5], color: [1.0, 0.4, 0.4] },
    Vertex { position: [-0.5,  0.5,  0.5], color: [1.0, 0.4, 0.4] },
    // Back face (green)
    Vertex { position: [-0.5, -0.5, -0.5], color: [0.4, 1.0, 0.4] },
    Vertex { position: [-0.5,  0.5, -0.5], color: [0.4, 1.0, 0.4] },
    Vertex { position: [ 0.5,  0.5, -0.5], color: [0.4, 1.0, 0.4] },
    Vertex { position: [ 0.5, -0.5, -0.5], color: [0.4, 1.0, 0.4] },
    // Top face (blue)
    Vertex { position: [-0.5,  0.5, -0.5], color: [0.4, 0.4, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], color: [0.4, 0.4, 1.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], color: [0.4, 0.4, 1.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], color: [0.4, 0.4, 1.0] },
    // Bottom face (yellow)
    Vertex { position: [-0.5, -0.5, -0.5], color: [1.0, 1.0, 0.4] },
    Vertex { position: [ 0.5, -0.5, -0.5], color: [1.0, 1.0, 0.4] },
    Vertex { position: [ 0.5, -0.5,  0.5], color: [1.0, 1.0, 0.4] },
    Vertex { position: [-0.5, -0.5,  0.5], color: [1.0, 1.0, 0.4] },
    // Right face (magenta)
    Vertex { position: [ 0.5, -0.5, -0.5], color: [1.0, 0.4, 1.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], color: [1.0, 0.4, 1.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], color: [1.0, 0.4, 1.0] },
    Vertex { position: [ 0.5, -0.5,  0.5], color: [1.0, 0.4, 1.0] },
    // Left face (cyan)
    Vertex { position: [-0.5, -0.5, -0.5], color: [0.4, 1.0, 1.0] },
    Vertex { position: [-0.5, -0.5,  0.5], color: [0.4, 1.0, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], color: [0.4, 1.0, 1.0] },
    Vertex { position: [-0.5,  0.5, -0.5], color: [0.4, 1.0, 1.0] },
];

const INDICES: &[u16] = &[
    0,  1,  2,  0,  2,  3,  // front
    4,  5,  6,  4,  6,  7,  // back
    8,  9,  10, 8,  10, 11, // top
    12, 13, 14, 12, 14, 15, // bottom
    16, 17, 18, 16, 18, 19, // right
    20, 21, 22, 20, 22, 23, // left
];

const SHADER: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
}
@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    rotation_x: f32,
    rotation_y: f32,
    mouse_pressed: bool,
    last_mouse_pos: Option<(f64, f64)>,
}

impl GpuState {
    async fn new(window: Arc<Window>) -> Self {
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[Uniforms { mvp: Mat4::IDENTITY.to_cols_array_2d() }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind Group Layout"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
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
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            window,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            rotation_x: 0.3,
            rotation_y: 0.0,
            mouse_pressed: false,
            last_mouse_pos: None,
        }
    }

    fn handle_mouse_input(&mut self, pressed: bool) {
        self.mouse_pressed = pressed;
        if !pressed {
            self.last_mouse_pos = None;
        }
    }

    fn handle_mouse_move(&mut self, x: f64, y: f64) {
        if self.mouse_pressed {
            if let Some((last_x, last_y)) = self.last_mouse_pos {
                let dx = (x - last_x) as f32;
                let dy = (y - last_y) as f32;
                
                self.apply_rotation(dx * 0.01, dy * 0.01);
                
                #[cfg(target_arch = "wasm32")]
                webrtc::send_rotation_to_peer(dx * 0.01, dy * 0.01);
            }
            self.last_mouse_pos = Some((x, y));
            self.window.request_redraw();
        }
    }
    
    fn apply_rotation(&mut self, dy: f32, dx: f32) {
        self.rotation_y += dy;
        self.rotation_x += dx;
        self.rotation_x = self.rotation_x.clamp(-1.5, 1.5);
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let aspect = self.config.width as f32 / self.config.height as f32;
        let projection = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y);
        let model = Mat4::from_rotation_y(self.rotation_y) * Mat4::from_rotation_x(self.rotation_x);
        let mvp = projection * view * model;

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms { mvp: mvp.to_cols_array_2d() }]),
        );

        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.1,
                            b: 0.15,
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

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static GPU_STATE: RefCell<Option<GpuState>> = const { RefCell::new(None) };
}

struct App {
    #[cfg(not(target_arch = "wasm32"))]
    gpu_state: Option<GpuState>,
}

impl App {
    fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            gpu_state: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.gpu_state.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title("Rust wgpu Cube");
        
        let window = Arc::new(event_loop.create_window(window_attrs).unwrap());

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

            let window_clone = window.clone();
            let init_width = width;
            let init_height = height;
            wasm_bindgen_futures::spawn_local(async move {
                let mut state = GpuState::new(window_clone.clone()).await;
                state.resize(winit::dpi::PhysicalSize::new(init_width, init_height));
                GPU_STATE.with(|s| {
                    *s.borrow_mut() = Some(state);
                });
                window_clone.request_redraw();
                
                // Initialize WebRTC client
                webrtc::init_webrtc_client();
                webrtc::set_rotation_callback(|dx, dy| {
                    GPU_STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            state.apply_rotation(dx, dy);
                            state.window.request_redraw();
                        }
                    });
                });
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.gpu_state = Some(pollster::block_on(GpuState::new(window.clone())));
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(physical_size) => {
                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.resize(physical_size);
                    }
                });
                
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = &mut self.gpu_state {
                    state.resize(physical_size);
                }
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Left, .. } => {
                let pressed = button_state == ElementState::Pressed;
                
                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.handle_mouse_input(pressed);
                    }
                });

                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = &mut self.gpu_state {
                    state.handle_mouse_input(pressed);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.handle_mouse_move(position.x, position.y);
                    }
                });

                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = &mut self.gpu_state {
                    state.handle_mouse_move(position.x, position.y);
                }
            }
            WindowEvent::RedrawRequested => {
                #[cfg(target_arch = "wasm32")]
                GPU_STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        match state.render() {
                            Ok(_) => {}
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

                #[cfg(not(target_arch = "wasm32"))]
                if let Some(state) = &mut self.gpu_state {
                    match state.render() {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) => state.resize(winit::dpi::PhysicalSize::new(
                            state.config.width,
                            state.config.height,
                        )),
                        Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                        Err(e) => log::error!("Render error: {:?}", e),
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("Couldn't initialize logger");
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
