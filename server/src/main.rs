mod signaling;
mod turn;

use axum::{Json, Router, routing::get};
use serde::Serialize;
use std::net::SocketAddr;
use tower_http::services::ServeDir;

const PORT: u16 = 9000;
const TURN_PORT: u16 = 3478;
const GIT_HASH: &str = env!("GIT_HASH");

#[tokio::main]
async fn main() {
    // Set log levels: info for most, but filter out noisy TURN "no allocation found" errors
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,turn::server=warn"),
    )
    .init();

    let public_ip = turn::get_public_ip();

    log::info!("=== Server version: {} ===", GIT_HASH);
    log::info!("Starting Rust game server...");
    log::info!("  HTTP+WS:   http://localhost:{}", PORT);
    log::info!("  TURN/STUN: turn:{}:{}", public_ip, TURN_PORT);

    // Start TURN/STUN server (UDP)
    tokio::spawn(async move {
        if let Err(e) = turn::run_turn_server(TURN_PORT, public_ip).await {
            log::error!("TURN server error: {}", e);
        }
    });

    // Start game loop
    signaling::start_game_loop();

    // Build router with WebSocket and static file serving
    let app = Router::new()
        .route("/ws", get(signaling::ws_handler))
        .route("/turn-credentials", get(turn_credentials))
        .fallback_service(ServeDir::new("client/dist"));

    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    log::info!("Server listening on {}", addr);

    axum::serve(listener, app).await.unwrap();
}

#[derive(Serialize)]
struct TurnCredential {
    urls: String,
    username: String,
    credential: String,
}

async fn turn_credentials() -> Json<Vec<TurnCredential>> {
    Json(vec![TurnCredential {
        urls: format!("turn:{{host}}:{}", TURN_PORT),
        username: turn::TURN_USERNAME.to_string(),
        credential: turn::TURN_PASSWORD.to_string(),
    }])
}
