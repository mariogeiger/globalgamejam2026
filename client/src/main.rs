use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

mod assets;
mod audio;
mod collision;
mod config;
mod game;
mod glb;
mod gpu;
mod input;
mod mesh;
mod network;
mod player;
mod render;

use assets::EMBEDDED_MAP;
use audio::Audio;
use config::{AFK_TIMEOUT_SECONDS, DEBUG_MANNEQUINS};
use game::{GameState, init_mask_images};
use glb::load_mesh_from_bytes;
use input::InputState;
use mesh::Mesh;
use network::NetworkClient;
use render::Renderer;

struct ClientState {
    renderer: Renderer,
    game: GameState,
    input: InputState,
    network: NetworkClient,
    audio: Audio,
}

struct App {
    state: Option<ClientState>,
    map_mesh: Option<Mesh>,
}

impl App {
    fn new() -> Self {
        // Load map with coordinate transform (rotate 180° around Z)
        let mut map_mesh = load_mesh_from_bytes(EMBEDDED_MAP).expect("Failed to load map");
        map_mesh.rotate_z_180();
        log::info!(
            "Loaded map: {} submeshes, {} textures",
            map_mesh.submeshes.len(),
            map_mesh.textures.len()
        );
        Self {
            state: None,
            map_mesh: Some(map_mesh),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
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

        let map_mesh = self.map_mesh.take().expect("Map already consumed");
        let window_clone = window.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let renderer = Renderer::new(window_clone.clone(), &map_mesh).await;
            let game = GameState::new(&map_mesh, DEBUG_MANNEQUINS);
            let input = InputState::new();
            let network = NetworkClient::new().expect("Failed to create network client");

            let audio = Audio::new();
            let state = ClientState {
                renderer,
                game,
                input,
                network,
                audio,
            };

            STATE.with(|s| *s.borrow_mut() = Some(state));
            init_mask_images();
            window_clone.request_redraw();
        });
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent) {
        let DeviceEvent::MouseMotion { delta } = event else {
            return;
        };
        STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let Some(state) = guard.as_mut() else { return };
            if state.input.cursor_grabbed {
                state
                    .input
                    .handle_mouse_move(delta.0 as f32, delta.1 as f32);
                state.renderer.request_redraw();
            }
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        state.renderer.resize(size.width, size.height);
                    }
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            match event.state {
                                ElementState::Pressed if key == KeyCode::Escape => {
                                    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
                                        d.exit_pointer_lock();
                                    }
                                    state.input.cursor_grabbed = false;
                                }
                                ElementState::Pressed => state.input.handle_key_press(key),
                                ElementState::Released => state.input.handle_key_release(key),
                            }
                            state.renderer.request_redraw();
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
                    STATE.with(|s| {
                        if let Some(state) = s.borrow_mut().as_mut() {
                            state.input.cursor_grabbed = true;
                        }
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        let scroll_y = match delta {
                            winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                            winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 100.0,
                        };
                        state.input.handle_scroll(scroll_y);
                        state.renderer.request_redraw();
                    }
                });
            }
            WindowEvent::RedrawRequested => {
                STATE.with(|s| {
                    if let Some(state) = s.borrow_mut().as_mut() {
                        // Check AFK timeout
                        if state.network.is_connected()
                            && state.input.seconds_since_activity() > AFK_TIMEOUT_SECONDS
                        {
                            log::info!("Disconnecting due to inactivity");
                            state.network.disconnect();
                            show_afk_overlay();
                        }

                        let local_peer_id = state.network.local_id();

                        for event in state.network.poll_events() {
                            state.game.handle_network_event(event, local_peer_id);
                        }

                        state.game.update(&mut state.input);

                        let (progress, has_target) = state.game.get_targeting_info();
                        state.audio.update_charge(has_target, progress);

                        for _ in 0..state.game.take_death_sounds() {
                            state.audio.play_death();
                        }

                        // Send any kills we made this frame
                        for victim_id in state.game.take_pending_kills() {
                            state.network.send_kill(victim_id);
                        }

                        // Notify server if we just died
                        if state.game.take_death_notification() {
                            state.network.notify_death();
                        }

                        if state.network.is_connected() && !state.game.is_dead {
                            state.network.send_player_state(
                                state.game.player.position,
                                state.game.player.yaw,
                                state.game.player.pitch,
                                state.game.player.mask as u8,
                            );
                        }

                        match state.renderer.render_frame(&state.game) {
                            Ok(_) => state.renderer.request_redraw(),
                            Err(wgpu::SurfaceError::Lost) => {
                                let (w, h) = (state.renderer.width(), state.renderer.height());
                                state.renderer.resize(w, h);
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                            Err(e) => log::error!("Render error: {:?}", e),
                        }

                        state.input.end_frame();
                    }
                });
            }
            _ => {}
        }
    }
}

use std::cell::RefCell;

thread_local! {
    static STATE: RefCell<Option<ClientState>> = const { RefCell::new(None) };
}

fn show_afk_overlay() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(overlay) = doc.get_element_by_id("afk-overlay")
    {
        let _ = overlay.set_attribute("style", "display: block;");
    }
}

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
    console_log::init_with_level(log::Level::Info).expect("Logger init failed");

    let event_loop = EventLoop::new().unwrap();
    #[allow(clippy::let_underscore_future)]
    let _ = event_loop.run_app(&mut App::new());
}

fn main() {
    run();
}
