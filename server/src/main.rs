/// Single Rust Server Binary
/// Serves static files, WebSocket signaling, and STUN
mod signaling;
mod stun;

use std::fs;
use std::path::Path;
use std::thread;
use tiny_http::{Header, Response, Server};

const HTTP_PORT: u16 = 8080;
const WS_PORT: u16 = 9000;
const STUN_PORT: u16 = 3478;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Starting Rust game server...");
    log::info!("  HTTP:      http://localhost:{}", HTTP_PORT);
    log::info!("  WebSocket: ws://localhost:{}", WS_PORT);
    log::info!("  STUN:      stun:localhost:{}", STUN_PORT);

    // Start STUN server
    thread::spawn(move || {
        if let Err(e) = stun::run_stun_server(STUN_PORT) {
            log::error!("STUN server error: {}", e);
        }
    });

    // Start WebSocket signaling server
    thread::spawn(move || {
        if let Err(e) = signaling::run_signaling_server(WS_PORT) {
            log::error!("Signaling server error: {}", e);
        }
    });

    // Run HTTP server on main thread
    run_http_server(HTTP_PORT);
}

fn run_http_server(port: u16) {
    let server = Server::http(format!("0.0.0.0:{}", port)).expect("Failed to start HTTP server");
    log::info!("HTTP server running on port {}", port);

    for request in server.incoming_requests() {
        let url_path = request.url().to_string();
        let file_path = get_file_path(&url_path);

        match fs::read(&file_path) {
            Ok(content) => {
                let content_type = get_content_type(&file_path);
                let header = Header::from_bytes("Content-Type", content_type).unwrap();
                let response = Response::from_data(content).with_header(header);
                let _ = request.respond(response);
            }
            Err(_) => {
                let response = Response::from_string("Not Found").with_status_code(404);
                let _ = request.respond(response);
            }
        }
    }
}

fn get_file_path(url_path: &str) -> String {
    let dist_dir = "dist";

    let clean_path = url_path.trim_start_matches('/');
    let clean_path = clean_path.split('?').next().unwrap_or(clean_path);

    if clean_path.is_empty() || clean_path == "/" {
        format!("{}/index.html", dist_dir)
    } else {
        let path = format!("{}/{}", dist_dir, clean_path);
        if Path::new(&path).is_dir() {
            format!("{}/index.html", path)
        } else {
            path
        }
    }
}

fn get_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("glb") => "model/gltf-binary",
        _ => "application/octet-stream",
    }
}
