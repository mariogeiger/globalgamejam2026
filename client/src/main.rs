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
mod debug;
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
use debug::DebugOverlay;
use game::{GameState, init_mask_images};
use glb::load_mesh_from_bytes;
use input::InputState;
use mesh::Mesh;
use network::NetworkClient;
use render::{Renderer, check_webgpu_support, show_webgpu_error};

struct ClientState {
    renderer: Renderer,
    game: GameState,
    input: InputState,
    network: Option<NetworkClient>,
    audio: Audio,
    player_name: Option<String>,
    debug: DebugOverlay,
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

        // Check WebGPU support before proceeding
        if !check_webgpu_support() {
            log::error!("WebGPU is not supported in this browser");
            show_webgpu_error();
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

            let audio = Audio::new();
            let debug = DebugOverlay::new();
            let state = ClientState {
                renderer,
                game,
                input,
                network: None,
                audio,
                player_name: None,
                debug,
            };

            STATE.with(|s| *s.borrow_mut() = Some(state));
            init_mask_images();
            setup_main_menu();
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
                // Only capture pointer if player has connected (entered their name)
                STATE.with(|s| {
                    let should_capture = s
                        .borrow()
                        .as_ref()
                        .map(|state| state.network.is_some())
                        .unwrap_or(false);

                    if should_capture
                        && let Some(canvas) = web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|d| d.get_element_by_id("wasm-container"))
                            .and_then(|c| c.first_element_child())
                    {
                        canvas.request_pointer_lock();
                        if let Some(state) = s.borrow_mut().as_mut() {
                            state.input.cursor_grabbed = true;
                        }
                    }
                });
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
                        state.debug.begin_frame();

                        // --- Network poll ---
                        state.debug.begin_section();
                        if let Some(ref network) = state.network {
                            if network.is_connected()
                                && state.input.seconds_since_activity() > AFK_TIMEOUT_SECONDS
                            {
                                log::info!("Disconnecting due to inactivity");
                                network.disconnect();
                                show_afk_overlay();
                            }

                            let local_peer_id = network.local_id();
                            for event in network.poll_events() {
                                state.game.handle_network_event(event, local_peer_id);
                            }
                        }
                        state.debug.end_net_poll();

                        // --- Game update ---
                        state.debug.begin_section();
                        state.game.update(&mut state.input, &mut state.debug);
                        state.debug.end_update();

                        let (progress, has_target) = state.game.get_targeting_info();
                        state.audio.update_charge(has_target, progress);

                        for _ in 0..state.game.take_death_sounds() {
                            state.audio.play_death();
                        }

                        // --- Network send ---
                        state.debug.begin_section();
                        if let Some(ref network) = state.network {
                            for victim_id in state.game.take_pending_kills() {
                                network.send_kill(victim_id);
                            }

                            if state.game.take_death_notification() {
                                network.notify_death();
                            }

                            if network.is_connected() && !state.game.is_dead {
                                network.send_player_state(
                                    state.game.player.position,
                                    state.game.player.yaw,
                                    state.game.player.pitch,
                                    state.game.player.mask as u8,
                                );
                            }

                            state.game.update_peer_stats(network);
                        }
                        state.debug.end_net_send();

                        // --- Render ---
                        state.debug.begin_section();
                        match state.renderer.render_frame(&state.game) {
                            Ok(_) => state.renderer.request_redraw(),
                            Err(wgpu::SurfaceError::Lost) => {
                                let (w, h) = (state.renderer.width(), state.renderer.height());
                                state.renderer.resize(w, h);
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                            Err(e) => log::error!("Render error: {:?}", e),
                        }
                        state.debug.end_render();

                        // --- Debug display update ---
                        let physics_debug = state.game.get_physics_debug();
                        state.debug.update_display(
                            state.game.player.position,
                            state.game.player.velocity,
                            &physics_debug,
                        );

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

fn random_french_name() -> String {
    use rand::Rng;
    const FIRST: &[&str] = &[
        "Jean",
        "Pierre",
        "Marie",
        "Louis",
        "François",
        "Antoine",
        "Jacques",
        "Michel",
        "André",
        "Philippe",
        "Alain",
        "Bernard",
        "Claude",
        "René",
        "Marcel",
        "Émile",
        "Céline",
        "Camille",
        "Léa",
        "Chloé",
        "Manon",
        "Inès",
        "Jade",
        "Zoé",
        "Lola",
        "Hugo",
        "Lucas",
        "Théo",
        "Enzo",
        "Mathis",
        "Nathan",
        "Maxime",
        "Julien",
    ];
    const LAST: &[&str] = &[
        "Martin", "Bernard", "Dubois", "Thomas", "Robert", "Richard", "Petit", "Durand", "Leroy",
        "Moreau", "Simon", "Laurent", "Lefebvre", "Michel", "Garcia", "David", "Bertrand", "Roux",
        "Vincent", "Fournier", "Morel", "Girard", "André", "Mercier",
    ];
    let mut rng = rand::rng();
    let first = FIRST[rng.random_range(0..FIRST.len())];
    let last = LAST[rng.random_range(0..LAST.len())];
    format!("{} {}", first, last)
}

fn setup_main_menu() {
    let doc = web_sys::window().and_then(|w| w.document()).unwrap();

    // Focus input, set random name, and handle Enter key
    if let Some(input) = doc.get_element_by_id("player-name-input") {
        let html_input: web_sys::HtmlInputElement = input.clone().unchecked_into();
        html_input.set_value(&random_french_name());
        html_input.select();
        let _ = html_input.focus();

        let cb = Closure::wrap(Box::new(|e: web_sys::KeyboardEvent| {
            if e.key() == "Enter" {
                start_game();
            }
        }) as Box<dyn FnMut(_)>);
        let _ = input.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // Handle start button click
    if let Some(btn) = doc.get_element_by_id("start-button") {
        let cb =
            Closure::wrap(Box::new(|_: web_sys::MouseEvent| start_game()) as Box<dyn FnMut(_)>);
        let _ = btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}

fn start_game() {
    let doc = web_sys::window().and_then(|w| w.document()).unwrap();
    let input: web_sys::HtmlInputElement = doc
        .get_element_by_id("player-name-input")
        .unwrap()
        .unchecked_into();
    let name = input.value().trim().to_string();

    if name.is_empty() {
        input.set_placeholder("Please enter a name!");
        return;
    }

    if let Some(menu) = doc.get_element_by_id("main-menu") {
        let _ = menu.set_attribute("style", "display: none;");
    }

    STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            state.player_name = Some(name.clone());
            state.game.set_local_name(name.clone());
            if let Ok(network) = NetworkClient::new(name) {
                state.network = Some(network);
            }
        }
    });
}

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

const GIT_HASH: &str = env!("GIT_HASH");

#[wasm_bindgen(start)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
    console_log::init_with_level(log::Level::Info).expect("Logger init failed");

    log::info!("=== Client version: {} ===", GIT_HASH);

    let event_loop = EventLoop::new().unwrap();
    #[allow(clippy::let_underscore_future)]
    let _ = event_loop.run_app(&mut App::new());
}

fn main() {
    run();
}
