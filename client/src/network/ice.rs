//! ICE server configuration and TURN credential fetching.
//!
//! Handles discovery and configuration of STUN/TURN servers:
//! - Self-hosted STUN server (based on current hostname)
//! - Self-hosted TURN server (credentials from /turn-credentials)
//! - Google STUN servers (fallback)
//! - Metered.ca TURN servers (fallback)

use super::protocol::PeerId;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Url;

/// ICE server configuration.
#[derive(Debug, Clone)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Metered.ca TURN server API endpoint.
const METERED_API_URL: &str = "https://ggj26.metered.live/api/v1/turn/credentials?apiKey=eb7440a97d22d69b25dfe8b64bbb3c79642f";

/// Get the signaling server WebSocket URL based on current page location.
pub fn signaling_server_url() -> String {
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href()
            && let Ok(url) = Url::new(&href)
        {
            let protocol = url.protocol();
            let host = url.host();
            let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
            return format!("{}//{}/ws", ws_protocol, host);
        }
    }
    "wss://localhost/ws".to_string()
}

/// Get base ICE servers (STUN only, no credentials needed).
pub fn base_ice_servers() -> Vec<IceServer> {
    let mut servers = vec![
        // Google STUN servers for NAT discovery
        IceServer {
            urls: vec![
                "stun:stun.l.google.com:19302".to_string(),
                "stun:stun1.l.google.com:19302".to_string(),
            ],
            username: None,
            credential: None,
        },
    ];

    // Add server's own STUN if not localhost
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href()
            && let Ok(url) = Url::new(&href)
        {
            let hostname = url.hostname();
            if !hostname.is_empty() && hostname != "localhost" {
                servers.insert(
                    0,
                    IceServer {
                        urls: vec![format!("stun:{}:3478", hostname)],
                        username: None,
                        credential: None,
                    },
                );
            }
        }
    }
    servers
}

/// Fetch TURN credentials from our own server.
pub async fn fetch_self_hosted_turn() -> Option<IceServer> {
    let window = web_sys::window()?;
    let location = window.location();
    let href = location.href().ok()?;
    let url = Url::new(&href).ok()?;
    let hostname = url.hostname();

    if hostname.is_empty() || hostname == "localhost" {
        return None;
    }

    let protocol = url.protocol();
    let creds_url = format!("{}//{}/turn-credentials", protocol, url.host());

    match JsFuture::from(window.fetch_with_str(&creds_url)).await {
        Ok(resp) => {
            let resp: web_sys::Response = resp.unchecked_into();
            if resp.ok()
                && let Ok(json) = JsFuture::from(resp.json().unwrap()).await
                && js_sys::Array::is_array(&json)
            {
                let arr = js_sys::Array::from(&json);
                let first = arr.get(0);
                if !first.is_undefined()
                    && let Some(urls) = js_sys::Reflect::get(&first, &"urls".into())
                        .ok()
                        .and_then(|v| v.as_string())
                {
                    let username = js_sys::Reflect::get(&first, &"username".into())
                        .ok()
                        .and_then(|v| v.as_string());
                    let credential = js_sys::Reflect::get(&first, &"credential".into())
                        .ok()
                        .and_then(|v| v.as_string());

                    // Replace {host} placeholder with actual hostname
                    let urls = urls.replace("{host}", &hostname);

                    let server = IceServer {
                        urls: vec![urls],
                        username,
                        credential,
                    };
                    log::info!("Fetched self-hosted TURN: {:?}", server.urls);
                    return Some(server);
                }
            }
        }
        Err(e) => {
            log::debug!("Self-hosted TURN fetch failed: {:?}", e);
        }
    }
    None
}

/// Fetch all available TURN servers (self-hosted + Metered.ca fallback).
pub async fn fetch_turn_servers(peer_id: PeerId) -> Vec<IceServer> {
    let mut servers = base_ice_servers();

    super::ui::net_log(
        super::ui::NetLogLevel::Info,
        &format!("Peer {}: Fetching TURN credentials...", peer_id),
    );

    // First try self-hosted TURN
    if let Some(self_hosted) = fetch_self_hosted_turn().await {
        servers.push(self_hosted);
        super::ui::net_log(
            super::ui::NetLogLevel::Success,
            &format!("Peer {}: Got self-hosted TURN", peer_id),
        );
    }

    // Then fetch TURN credentials from Metered.ca API as fallback
    if let Some(window) = web_sys::window() {
        match JsFuture::from(window.fetch_with_str(METERED_API_URL)).await {
            Ok(resp) => {
                let resp: web_sys::Response = resp.unchecked_into();
                if resp.ok() {
                    if let Ok(json) = JsFuture::from(resp.json().unwrap()).await
                        && js_sys::Array::is_array(&json)
                    {
                        let arr = js_sys::Array::from(&json);
                        let mut turn_count = 0;
                        for i in 0..arr.length() {
                            let server = arr.get(i);
                            if let Some(urls) = js_sys::Reflect::get(&server, &"urls".into())
                                .ok()
                                .and_then(|v| v.as_string())
                            {
                                let username = js_sys::Reflect::get(&server, &"username".into())
                                    .ok()
                                    .and_then(|v| v.as_string());
                                let credential =
                                    js_sys::Reflect::get(&server, &"credential".into())
                                        .ok()
                                        .and_then(|v| v.as_string());

                                log::info!(
                                    "Fetched TURN server: {} (has creds: {})",
                                    urls,
                                    username.is_some()
                                );

                                if urls.starts_with("turn") {
                                    turn_count += 1;
                                }

                                servers.push(IceServer {
                                    urls: vec![urls],
                                    username,
                                    credential,
                                });
                            }
                        }
                        super::ui::net_log(
                            super::ui::NetLogLevel::Success,
                            &format!("Peer {}: Got {} fallback TURN servers", peer_id, turn_count),
                        );
                    }
                } else {
                    let msg = format!("TURN API error: {}", resp.status());
                    log::warn!("{}", msg);
                    super::ui::net_log(super::ui::NetLogLevel::Error, &msg);
                }
            }
            Err(e) => {
                let msg = format!("TURN fetch failed: {:?}", e);
                log::warn!("{}", msg);
                super::ui::net_log(super::ui::NetLogLevel::Error, "TURN fetch failed");
            }
        }
    }

    servers
}

/// Convert ICE servers to a JS array for RtcConfiguration.
pub fn to_js_ice_servers(servers: &[IceServer]) -> Result<js_sys::Array, wasm_bindgen::JsValue> {
    let ice_servers_array = js_sys::Array::new();

    for ice_server in servers {
        let urls = js_sys::Array::new();
        for url in &ice_server.urls {
            urls.push(&url.clone().into());
        }
        let server = js_sys::Object::new();
        js_sys::Reflect::set(&server, &"urls".into(), &urls)?;

        if let Some(ref username) = ice_server.username {
            js_sys::Reflect::set(&server, &"username".into(), &username.clone().into())?;
        }
        if let Some(ref credential) = ice_server.credential {
            js_sys::Reflect::set(&server, &"credential".into(), &credential.clone().into())?;
        }

        ice_servers_array.push(&server);
    }

    Ok(ice_servers_array)
}
