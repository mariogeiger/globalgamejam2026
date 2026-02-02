mod signaling;
mod stun;

use axum::{Router, routing::get};
use std::net::SocketAddr;
use tower_http::services::ServeDir;

const PORT: u16 = 9000;
const STUN_PORT: u16 = 3478;
const GIT_HASH: &str = env!("GIT_HASH");

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("=== Server version: {} ===", GIT_HASH);
    log::info!("Starting Rust game server...");
    log::info!("  HTTP+WS:   http://localhost:{}", PORT);
    log::info!("  STUN:      stun:localhost:{}", STUN_PORT);

    // Start STUN server (UDP, runs in blocking thread)
    tokio::spawn(async move {
        if let Err(e) = stun::run_stun_server(STUN_PORT).await {
            log::error!("STUN server error: {}", e);
        }
    });

    // Start game loop
    signaling::start_game_loop();

    // Build router with WebSocket and static file serving
    let app = Router::new()
        .route("/ws", get(signaling::ws_handler))
        .fallback_service(ServeDir::new("client/dist"));

    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    log::info!("Server listening on {}", addr);

    axum::serve(listener, app).await.unwrap();
}
