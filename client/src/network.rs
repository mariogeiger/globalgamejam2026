use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent, RtcDataChannelInit,
    RtcIceCandidate, RtcIceCandidateInit, RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType,
    RtcSessionDescriptionInit, Url, WebSocket,
};

pub type PeerId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    WaitingForPlayers,
    GracePeriod,
    Playing,
    Victory,
    Spectating,
}

#[derive(Clone, Debug)]
pub enum NetworkEvent {
    Connected {
        id: PeerId,
        phase: GamePhase,
        phase_time_remaining: f32,
    },
    PeerJoined {
        id: PeerId,
    },
    PeerLeft {
        id: PeerId,
    },
    GamePhaseChanged {
        phase: GamePhase,
        time_remaining: f32,
    },
    PlayerState {
        id: PeerId,
        position: Vec3,
        yaw: f32,
        pitch: f32,
        mask: u8,
    },
    PlayerKilled {
        killer_id: PeerId,
        victim_id: PeerId,
    },
    PeerIntroduction {
        id: PeerId,
        name: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlayerStateMessage {
    msg_type: String,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    #[serde(default)]
    pitch: f32,
    #[serde(default)]
    mask: u8,
}

impl PlayerStateMessage {
    fn new(position: Vec3, yaw: f32, pitch: f32, mask: u8) -> Self {
        Self {
            msg_type: "player_state".to_string(),
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
            mask,
        }
    }
}

/// Messages sent/received on the reliable "events" channel
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum EventMessage {
    #[serde(rename = "kill")]
    Kill { victim_id: PeerId },
    #[serde(rename = "introduction")]
    Introduction { name: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PeerInfo {
    id: PeerId,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "welcome")]
    Welcome {
        #[serde(rename = "clientId")]
        client_id: PeerId,
        peers: Vec<PeerInfo>,
        #[serde(rename = "gamePhase")]
        game_phase: GamePhase,
        #[serde(rename = "phaseTimeRemaining")]
        phase_time_remaining: f32,
    },
    #[serde(rename = "peer-joined")]
    PeerJoined {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
    },
    #[serde(rename = "peer-left")]
    PeerLeft {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
    },
    #[serde(rename = "game-phase")]
    GamePhase {
        phase: GamePhase,
        #[serde(rename = "timeRemaining")]
        time_remaining: f32,
    },
    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "fromId")]
        from_id: PeerId,
        sdp: String,
    },
    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "fromId")]
        from_id: PeerId,
        sdp: String,
    },
    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "fromId")]
        from_id: PeerId,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
    },
}

#[allow(dead_code)]
struct PeerConnection {
    pc: RtcPeerConnection,
    /// Unreliable channel for position updates
    state_channel: Option<RtcDataChannel>,
    /// Reliable channel for game events (kills)
    events_channel: Option<RtcDataChannel>,
    pending_candidates: Vec<IceCandidateData>,
}

#[derive(Clone)]
struct IceCandidateData {
    candidate: String,
    sdp_mid: Option<String>,
    sdp_m_line_index: Option<u16>,
}

/// Connection type based on ICE candidate type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionType {
    /// Direct peer-to-peer connection (host candidate)
    Direct,
    /// Connection via STUN (server-reflexive or peer-reflexive)
    Stun,
    /// Connection via TURN relay
    Turn,
    /// Unknown/connecting
    Unknown,
}

/// Statistics for a single peer connection
#[derive(Clone, Debug)]
pub struct PeerStats {
    pub peer_id: PeerId,
    pub name: Option<String>,
    pub connection_type: ConnectionType,
    /// Round-trip time in milliseconds, if available
    pub rtt_ms: Option<f64>,
}

#[derive(Default)]
struct RtcState {
    local_id: Option<PeerId>,
    local_name: String,
    peers: HashMap<PeerId, PeerConnection>,
    pending_events: Vec<NetworkEvent>,
}

type StateRef = Rc<RefCell<RtcState>>;

pub struct NetworkClient {
    state: StateRef,
    ws: WebSocket,
}

impl NetworkClient {
    pub fn new(player_name: String) -> Result<Self, JsValue> {
        let state: StateRef = Rc::new(RefCell::new(RtcState {
            local_name: player_name,
            ..Default::default()
        }));
        let server_url = signaling_server_url();
        log::info!("Connecting to signaling server: {}", server_url);
        let ws = WebSocket::new(&server_url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let ws_clone = ws.clone();
        set_callback(&ws, "onopen", move |_: JsValue| {
            log::info!("Connected to signaling server");
            net_log(NetLogLevel::Success, "Connected to signaling server");
            let _ = ws_clone.send_with_str(r#"{"type":"join"}"#);
        });

        let state_clone = state.clone();
        let ws_clone = ws.clone();
        set_callback(&ws, "onmessage", move |ev: MessageEvent| {
            if let Some(text) = ev.data().as_string() {
                if let Ok(msg) = serde_json::from_str::<ServerMessage>(&text) {
                    handle_signal(&ws_clone, &state_clone, msg);
                } else {
                    log::warn!("Failed to parse server message: {}", text);
                }
            }
        });

        set_callback(&ws, "onerror", |_: JsValue| {
            log::error!("WebSocket error");
            net_log(NetLogLevel::Error, "WebSocket error - server unreachable?");
        });
        set_callback(&ws, "onclose", |_: JsValue| {
            log::info!("WebSocket closed");
            net_log(NetLogLevel::Warning, "Disconnected from server");
        });

        Ok(Self { state, ws })
    }

    pub fn poll_events(&self) -> Vec<NetworkEvent> {
        let mut state = self.state.borrow_mut();
        std::mem::take(&mut state.pending_events)
    }

    pub fn send_player_state(&self, position: Vec3, yaw: f32, pitch: f32, mask: u8) {
        let state = self.state.borrow();
        if let Ok(json) =
            serde_json::to_string(&PlayerStateMessage::new(position, yaw, pitch, mask))
        {
            let mut sent_count = 0;
            let mut skipped = Vec::new();
            for (&peer_id, peer) in &state.peers {
                if let Some(ref dc) = peer.state_channel {
                    if dc.ready_state() == web_sys::RtcDataChannelState::Open {
                        let _ = dc.send_with_str(&json);
                        sent_count += 1;
                    } else {
                        skipped.push((peer_id, format!("{:?}", dc.ready_state())));
                    }
                } else {
                    skipped.push((peer_id, "no channel".to_string()));
                }
            }
            if sent_count > 0 || !skipped.is_empty() {
                log::debug!(
                    "Sent state to {} peers, skipped: {:?}, pos: [{:.1}, {:.1}, {:.1}]",
                    sent_count,
                    skipped,
                    position.x,
                    position.y,
                    position.z
                );
            }
        }
    }

    pub fn send_kill(&self, victim_id: PeerId) {
        let state = self.state.borrow();
        let msg = EventMessage::Kill { victim_id };
        if let Ok(json) = serde_json::to_string(&msg) {
            for peer in state.peers.values() {
                if let Some(ref dc) = peer.events_channel
                    && dc.ready_state() == web_sys::RtcDataChannelState::Open
                {
                    let _ = dc.send_with_str(&json);
                }
            }
        }
    }

    pub fn local_id(&self) -> Option<PeerId> {
        self.state.borrow().local_id
    }

    pub fn is_connected(&self) -> bool {
        self.state.borrow().local_id.is_some()
    }

    pub fn notify_death(&self) {
        let _ = self.ws.send_with_str(r#"{"type":"player_died"}"#);
    }

    pub fn disconnect(&self) {
        let _ = self.ws.send_with_str(r#"{"type":"leave"}"#);
        let _ = self.ws.close();
    }

    /// Get the list of connected peer IDs with their RTCPeerConnection references
    /// Used for collecting stats
    pub fn get_peer_connections(&self) -> Vec<(PeerId, RtcPeerConnection)> {
        let state = self.state.borrow();
        state
            .peers
            .iter()
            .filter(|(_, peer)| {
                peer.state_channel
                    .as_ref()
                    .is_some_and(|dc| dc.ready_state() == web_sys::RtcDataChannelState::Open)
            })
            .map(|(&id, peer)| (id, peer.pc.clone()))
            .collect()
    }
}

fn signaling_server_url() -> String {
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

struct IceServer {
    urls: Vec<String>,
    username: Option<String>,
    credential: Option<String>,
}

// Metered.ca TURN server API
const METERED_API_URL: &str = "https://ggj26.metered.live/api/v1/turn/credentials?apiKey=eb7440a97d22d69b25dfe8b64bbb3c79642f";

fn base_ice_servers() -> Vec<IceServer> {
    let mut servers = vec![
        // STUN servers for NAT discovery
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

/// Fetch TURN credentials from our own server
async fn fetch_self_hosted_turn() -> Option<IceServer> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

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

async fn fetch_turn_servers(peer_id: PeerId) -> Vec<IceServer> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let mut servers = base_ice_servers();
    net_log(
        NetLogLevel::Info,
        &format!("Peer {}: Fetching TURN credentials...", peer_id),
    );

    // First try self-hosted TURN
    if let Some(self_hosted) = fetch_self_hosted_turn().await {
        servers.push(self_hosted);
        net_log(
            NetLogLevel::Success,
            &format!("Peer {}: Got self-hosted TURN", peer_id),
        );
    }

    // Then fetch TURN credentials from Metered.ca API as fallback
    if let Some(window) = web_sys::window() {
        match JsFuture::from(window.fetch_with_str(METERED_API_URL)).await {
            Ok(resp) => {
                let resp: web_sys::Response = resp.unchecked_into();
                if resp.ok() {
                    if let Ok(json) = JsFuture::from(resp.json().unwrap()).await {
                        // Parse the array of ICE servers
                        if js_sys::Array::is_array(&json) {
                            let arr = js_sys::Array::from(&json);
                            let mut turn_count = 0;
                            for i in 0..arr.length() {
                                let server = arr.get(i);
                                if let Some(urls) = js_sys::Reflect::get(&server, &"urls".into())
                                    .ok()
                                    .and_then(|v| v.as_string())
                                {
                                    let username =
                                        js_sys::Reflect::get(&server, &"username".into())
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
                            net_log(
                                NetLogLevel::Success,
                                &format!(
                                    "Peer {}: Got {} fallback TURN servers",
                                    peer_id, turn_count
                                ),
                            );
                        }
                    }
                } else {
                    let msg = format!("TURN API error: {}", resp.status());
                    log::warn!("{}", msg);
                    net_log(NetLogLevel::Error, &msg);
                }
            }
            Err(e) => {
                let msg = format!("TURN fetch failed: {:?}", e);
                log::warn!("{}", msg);
                net_log(NetLogLevel::Error, "TURN fetch failed");
            }
        }
    }

    servers
}

fn handle_signal(ws: &WebSocket, state: &StateRef, msg: ServerMessage) {
    let ws = ws.clone();
    let state = state.clone();

    wasm_bindgen_futures::spawn_local(async move {
        match msg {
            ServerMessage::Welcome {
                client_id,
                peers,
                game_phase,
                phase_time_remaining,
            } => {
                log::info!(
                    "Welcome! I am client {}, {} peers in game, phase: {:?}",
                    client_id,
                    peers.len(),
                    game_phase
                );

                {
                    let mut s = state.borrow_mut();
                    s.local_id = Some(client_id);
                    s.pending_events.push(NetworkEvent::Connected {
                        id: client_id,
                        phase: game_phase,
                        phase_time_remaining,
                    });
                }

                for peer_info in peers {
                    if let Ok(pc) = create_peer_connection(&ws, &state, peer_info.id).await {
                        // Create unreliable channel for position updates
                        let state_init = RtcDataChannelInit::new();
                        state_init.set_ordered(false);
                        state_init.set_max_retransmits(0);
                        let state_dc =
                            pc.create_data_channel_with_data_channel_dict("state", &state_init);
                        setup_state_channel(&state_dc, &state, peer_info.id);

                        // Create reliable channel for game events (kills)
                        let events_dc = pc.create_data_channel("events");
                        setup_events_channel(&events_dc, &state, peer_info.id);

                        {
                            let mut s = state.borrow_mut();
                            s.peers.insert(
                                peer_info.id,
                                PeerConnection {
                                    pc: pc.clone(),
                                    state_channel: Some(state_dc),
                                    events_channel: Some(events_dc),
                                    pending_candidates: Vec::new(),
                                },
                            );
                            s.pending_events
                                .push(NetworkEvent::PeerJoined { id: peer_info.id });
                        }

                        if let Ok(offer) =
                            wasm_bindgen_futures::JsFuture::from(pc.create_offer()).await
                        {
                            let sdp = get_sdp(&offer);
                            let init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                            init.set_sdp(&sdp);
                            if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&init))
                                .await
                                .is_ok()
                            {
                                send_offer(&ws, peer_info.id, &sdp);
                            }
                        }
                    }
                }
            }
            ServerMessage::PeerJoined { peer_id } => {
                log::info!("Peer {} joined", peer_id);
                net_log(NetLogLevel::Info, &format!("Peer {}: Joined", peer_id));

                if let Ok(pc) = create_peer_connection(&ws, &state, peer_id).await {
                    let mut s = state.borrow_mut();
                    s.peers.insert(
                        peer_id,
                        PeerConnection {
                            pc,
                            state_channel: None,
                            events_channel: None,
                            pending_candidates: Vec::new(),
                        },
                    );
                    s.pending_events
                        .push(NetworkEvent::PeerJoined { id: peer_id });
                }
            }
            ServerMessage::PeerLeft { peer_id } => {
                log::info!("Peer {} left", peer_id);
                net_log(NetLogLevel::Warning, &format!("Peer {}: Left", peer_id));
                {
                    let mut s = state.borrow_mut();
                    if let Some(peer) = s.peers.remove(&peer_id) {
                        peer.pc.close();
                    }
                    s.pending_events
                        .push(NetworkEvent::PeerLeft { id: peer_id });
                }
            }
            ServerMessage::GamePhase {
                phase,
                time_remaining,
            } => {
                log::info!(
                    "Game phase changed to {:?}, time: {}",
                    phase,
                    time_remaining
                );
                let mut s = state.borrow_mut();
                s.pending_events.push(NetworkEvent::GamePhaseChanged {
                    phase,
                    time_remaining,
                });
            }
            ServerMessage::Offer { from_id, sdp } => {
                log::info!("Received offer from peer {}", from_id);
                let pc = state.borrow().peers.get(&from_id).map(|p| p.pc.clone());

                if let Some(pc) = pc {
                    let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                    desc.set_sdp(&sdp);
                    if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc))
                        .await
                        .is_ok()
                    {
                        log::info!(
                            "Set remote description for peer {}, creating answer...",
                            from_id
                        );
                        apply_pending_candidates(&pc, &state, from_id).await;
                        if let Ok(answer) =
                            wasm_bindgen_futures::JsFuture::from(pc.create_answer()).await
                        {
                            let answer_sdp = get_sdp(&answer);
                            let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                            init.set_sdp(&answer_sdp);
                            if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&init))
                                .await
                                .is_ok()
                            {
                                log::info!("Sending answer to peer {}", from_id);
                                send_answer(&ws, from_id, &answer_sdp);
                            }
                        }
                    } else {
                        log::warn!("Failed to set remote description from peer {}", from_id);
                    }
                } else {
                    log::warn!("Received offer from unknown peer {}", from_id);
                }
            }
            ServerMessage::Answer { from_id, sdp } => {
                log::info!("Received answer from peer {}", from_id);
                let pc = state.borrow().peers.get(&from_id).map(|p| p.pc.clone());
                if let Some(pc) = pc {
                    let desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                    desc.set_sdp(&sdp);
                    if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc))
                        .await
                        .is_ok()
                    {
                        log::info!(
                            "Set remote description for peer {}, ice state: {:?}",
                            from_id,
                            pc.ice_connection_state()
                        );
                        apply_pending_candidates(&pc, &state, from_id).await;
                    }
                } else {
                    log::warn!("Received answer from unknown peer {}", from_id);
                }
            }
            ServerMessage::IceCandidate {
                from_id,
                candidate,
                sdp_mid,
                sdp_m_line_index,
            } => {
                log::info!(
                    "Received ICE candidate from peer {}: {}",
                    from_id,
                    candidate
                );
                let (has_remote, pc) = {
                    let s = state.borrow();
                    if let Some(peer) = s.peers.get(&from_id) {
                        (
                            peer.pc.remote_description().is_some(),
                            Some(peer.pc.clone()),
                        )
                    } else {
                        log::warn!("Received ICE candidate from unknown peer {}", from_id);
                        (false, None)
                    }
                };

                if has_remote {
                    if let Some(pc) = pc {
                        log::info!("Applying ICE candidate from peer {} immediately", from_id);
                        add_ice_candidate(
                            &pc,
                            &IceCandidateData {
                                candidate,
                                sdp_mid,
                                sdp_m_line_index,
                            },
                        )
                        .await;
                    }
                } else {
                    log::info!(
                        "Queueing ICE candidate from peer {} (no remote description yet)",
                        from_id
                    );
                    let mut s = state.borrow_mut();
                    if let Some(peer) = s.peers.get_mut(&from_id) {
                        peer.pending_candidates.push(IceCandidateData {
                            candidate,
                            sdp_mid,
                            sdp_m_line_index,
                        });
                    }
                }
            }
        }
    });
}

async fn create_peer_connection(
    ws: &WebSocket,
    state: &StateRef,
    peer_id: PeerId,
) -> Result<RtcPeerConnection, JsValue> {
    let config = RtcConfiguration::new();
    let ice_servers_array = js_sys::Array::new();

    // Fetch TURN servers from Metered.ca API
    let servers = fetch_turn_servers(peer_id).await;

    for ice_server in &servers {
        let urls = js_sys::Array::new();
        for url in &ice_server.urls {
            urls.push(&url.clone().into());
        }
        let server = js_sys::Object::new();
        js_sys::Reflect::set(&server, &"urls".into(), &urls)?;

        // Add credentials for TURN servers
        if let Some(ref username) = ice_server.username {
            js_sys::Reflect::set(&server, &"username".into(), &username.clone().into())?;
        }
        if let Some(ref credential) = ice_server.credential {
            js_sys::Reflect::set(&server, &"credential".into(), &credential.clone().into())?;
        }

        ice_servers_array.push(&server);
    }

    // Log configured ICE servers
    for server in &servers {
        log::info!(
            "ICE server: {:?} (has credentials: {})",
            server.urls,
            server.username.is_some()
        );
    }
    log::info!(
        "Configured {} ICE servers for peer {}",
        ice_servers_array.length(),
        peer_id
    );
    config.set_ice_servers(&ice_servers_array);

    let pc = RtcPeerConnection::new_with_configuration(&config)?;

    let ws_clone = ws.clone();
    let onicecandidate = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
        if let Some(c) = ev.candidate() {
            log::info!(
                "Sending ICE candidate to peer {}: {}",
                peer_id,
                c.candidate()
            );
            let msg = serde_json::json!({
                "type": "ice-candidate",
                "targetId": peer_id,
                "candidate": c.candidate(),
                "sdpMid": c.sdp_mid(),
                "sdpMLineIndex": c.sdp_m_line_index()
            });
            let _ = ws_clone.send_with_str(&msg.to_string());
        } else {
            log::info!("ICE gathering complete for peer {}", peer_id);
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_onicecandidate(Some(onicecandidate.as_ref().unchecked_ref()));
    onicecandidate.forget();

    let onice = Closure::wrap(Box::new(move |ev: JsValue| {
        if let Some(pc) = ev.dyn_ref::<RtcPeerConnection>() {
            let ice_state = pc.ice_connection_state();
            let gathering_state = pc.ice_gathering_state();
            log::info!(
                "Peer {} ICE state: {:?}, gathering: {:?}",
                peer_id,
                ice_state,
                gathering_state
            );
            match ice_state {
                web_sys::RtcIceConnectionState::New => {
                    net_log(
                        NetLogLevel::Info,
                        &format!("Peer {}: ICE starting", peer_id),
                    );
                }
                web_sys::RtcIceConnectionState::Checking => {
                    net_log(
                        NetLogLevel::Info,
                        &format!("Peer {}: Trying connection...", peer_id),
                    );
                }
                web_sys::RtcIceConnectionState::Connected => {
                    log::info!("Peer {} CONNECTED!", peer_id);
                    net_log(
                        NetLogLevel::Success,
                        &format!("Peer {}: Connected!", peer_id),
                    );
                }
                web_sys::RtcIceConnectionState::Failed => {
                    log::error!("Peer {} connection FAILED!", peer_id);
                    net_log(
                        NetLogLevel::Error,
                        &format!("Peer {}: Connection failed", peer_id),
                    );
                }
                web_sys::RtcIceConnectionState::Disconnected => {
                    log::warn!("Peer {} disconnected", peer_id);
                    net_log(
                        NetLogLevel::Warning,
                        &format!("Peer {}: Disconnected", peer_id),
                    );
                }
                _ => {}
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_oniceconnectionstatechange(Some(onice.as_ref().unchecked_ref()));
    onice.forget();

    let state_clone = state.clone();
    let ondc = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcDataChannelEvent = ev.unchecked_into();
        let dc = ev.channel();
        let label = dc.label();
        log::info!(
            "Received data channel '{}' from peer {}, state: {:?}",
            label,
            peer_id,
            dc.ready_state()
        );

        match label.as_str() {
            "state" => {
                setup_state_channel(&dc, &state_clone, peer_id);
                let mut s = state_clone.borrow_mut();
                if let Some(peer) = s.peers.get_mut(&peer_id) {
                    peer.state_channel = Some(dc);
                    log::info!("State channel stored for peer {}", peer_id);
                } else {
                    log::warn!("Peer {} not found when storing state channel!", peer_id);
                }
            }
            "events" => {
                setup_events_channel(&dc, &state_clone, peer_id);
                let mut s = state_clone.borrow_mut();
                if let Some(peer) = s.peers.get_mut(&peer_id) {
                    peer.events_channel = Some(dc);
                    log::info!("Events channel stored for peer {}", peer_id);
                } else {
                    log::warn!("Peer {} not found when storing events channel!", peer_id);
                }
            }
            _ => {
                log::warn!("Unknown data channel: {}", label);
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_ondatachannel(Some(ondc.as_ref().unchecked_ref()));
    ondc.forget();

    Ok(pc)
}

/// Set up the unreliable "state" channel for position updates
fn setup_state_channel(dc: &RtcDataChannel, state: &StateRef, peer_id: PeerId) {
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("State channel open with peer {}", peer_id);
        net_log(
            NetLogLevel::Success,
            &format!("Peer {}: Data channel ready", peer_id),
        );
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let state_clone = state.clone();
    let onmsg = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(data) = ev.data().as_string() else {
            log::warn!(
                "State channel: received non-string data from peer {}",
                peer_id
            );
            return;
        };
        let Ok(msg) = serde_json::from_str::<PlayerStateMessage>(&data) else {
            log::warn!(
                "State channel: failed to parse message from peer {}: {}",
                peer_id,
                data
            );
            return;
        };

        log::debug!(
            "Received state from peer {}: pos=[{:.1}, {:.1}, {:.1}]",
            peer_id,
            msg.x,
            msg.y,
            msg.z
        );

        let mut s = state_clone.borrow_mut();
        s.pending_events.push(NetworkEvent::PlayerState {
            id: peer_id,
            position: Vec3::new(msg.x, msg.y, msg.z),
            yaw: msg.yaw,
            pitch: msg.pitch,
            mask: msg.mask,
        });
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
    onmsg.forget();
}

/// Set up the reliable "events" channel for game events (kills, introductions)
fn setup_events_channel(dc: &RtcDataChannel, state: &StateRef, peer_id: PeerId) {
    let state_clone = state.clone();
    let dc_clone = dc.clone();
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("Events channel open with peer {}", peer_id);
        // Send our introduction to the peer
        let name = state_clone.borrow().local_name.clone();
        let msg = EventMessage::Introduction { name };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = dc_clone.send_with_str(&json);
        }
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let state_clone = state.clone();
    let onmsg = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(data) = ev.data().as_string() else {
            return;
        };
        let Ok(msg) = serde_json::from_str::<EventMessage>(&data) else {
            log::warn!("Failed to parse event message: {}", data);
            return;
        };

        match msg {
            EventMessage::Kill { victim_id } => {
                log::info!("Received kill event: peer {} killed {}", peer_id, victim_id);
                let mut s = state_clone.borrow_mut();
                s.pending_events.push(NetworkEvent::PlayerKilled {
                    killer_id: peer_id,
                    victim_id,
                });
            }
            EventMessage::Introduction { name } => {
                log::info!("Peer {} introduced as '{}'", peer_id, name);
                let mut s = state_clone.borrow_mut();
                s.pending_events
                    .push(NetworkEvent::PeerIntroduction { id: peer_id, name });
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
    onmsg.forget();
}

async fn apply_pending_candidates(pc: &RtcPeerConnection, state: &StateRef, peer_id: PeerId) {
    let pending: Vec<IceCandidateData> = {
        let mut s = state.borrow_mut();
        if let Some(peer) = s.peers.get_mut(&peer_id) {
            peer.pending_candidates.drain(..).collect()
        } else {
            Vec::new()
        }
    };
    log::info!(
        "Applying {} pending ICE candidates for peer {}",
        pending.len(),
        peer_id
    );
    for ice in pending {
        add_ice_candidate(pc, &ice).await;
    }
}

async fn add_ice_candidate(pc: &RtcPeerConnection, ice: &IceCandidateData) {
    let init = RtcIceCandidateInit::new(&ice.candidate);
    if let Some(ref mid) = ice.sdp_mid {
        init.set_sdp_mid(Some(mid));
    }
    if let Some(idx) = ice.sdp_m_line_index {
        init.set_sdp_m_line_index(Some(idx));
    }
    if let Ok(candidate) = RtcIceCandidate::new(&init) {
        let _ = wasm_bindgen_futures::JsFuture::from(
            pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&candidate)),
        )
        .await;
    }
}

fn get_sdp(js: &JsValue) -> String {
    js_sys::Reflect::get(js, &"sdp".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

fn send_offer(ws: &WebSocket, target_id: PeerId, sdp: &str) {
    let msg = serde_json::json!({ "type": "offer", "targetId": target_id, "sdp": sdp });
    let _ = ws.send_with_str(&msg.to_string());
}

fn send_answer(ws: &WebSocket, target_id: PeerId, sdp: &str) {
    let msg = serde_json::json!({ "type": "answer", "targetId": target_id, "sdp": sdp });
    let _ = ws.send_with_str(&msg.to_string());
}

fn set_callback<T: wasm_bindgen::convert::FromWasmAbi + 'static, F: FnMut(T) + 'static>(
    ws: &WebSocket,
    event: &str,
    mut f: F,
) {
    let closure = Closure::wrap(Box::new(move |e: T| f(e)) as Box<dyn FnMut(T)>);
    match event {
        "onopen" => ws.set_onopen(Some(closure.as_ref().unchecked_ref())),
        "onmessage" => ws.set_onmessage(Some(closure.as_ref().unchecked_ref())),
        "onerror" => ws.set_onerror(Some(closure.as_ref().unchecked_ref())),
        "onclose" => ws.set_onclose(Some(closure.as_ref().unchecked_ref())),
        _ => {}
    }
    closure.forget();
}

/// Log level for network status messages
#[derive(Clone, Copy)]
enum NetLogLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Add a line to the network status log in the UI
fn net_log(level: NetLogLevel, msg: &str) {
    let class = match level {
        NetLogLevel::Info => "info",
        NetLogLevel::Success => "success",
        NetLogLevel::Warning => "warning",
        NetLogLevel::Error => "error",
    };

    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(container) = doc.get_element_by_id("network-status")
        && let Ok(div) = doc.create_element("div")
    {
        let _ = div.set_attribute("class", &format!("log-line {}", class));
        div.set_text_content(Some(msg));
        let _ = container.append_child(&div);
        // Auto-scroll to bottom
        container.set_scroll_top(container.scroll_height());
    }
}

/// Fetch stats from a single peer connection and parse the RTCStatsReport
pub async fn fetch_peer_stats(
    peer_id: PeerId,
    name: Option<String>,
    pc: RtcPeerConnection,
) -> Option<PeerStats> {
    use wasm_bindgen_futures::JsFuture;

    // get_stats() returns a Promise<RTCStatsReport>
    let stats_promise = pc.get_stats();
    let stats_report = match JsFuture::from(stats_promise).await {
        Ok(report) => report,
        Err(e) => {
            log::debug!("Failed to get stats for peer {}: {:?}", peer_id, e);
            return None;
        }
    };

    // RTCStatsReport is a JS Map - iterate through it
    let stats_map: js_sys::Map = stats_report.unchecked_into();

    let mut connection_type = ConnectionType::Unknown;
    let mut rtt_ms: Option<f64> = None;
    let mut local_candidate_id: Option<String> = None;

    // First pass: find the succeeded candidate-pair
    stats_map.for_each(&mut |value, _key| {
        if let Ok(obj) = js_sys::Reflect::get(&value, &"type".into())
            && let Some(type_str) = obj.as_string()
            && type_str == "candidate-pair"
            && let Ok(state) = js_sys::Reflect::get(&value, &"state".into())
            && state.as_string().as_deref() == Some("succeeded")
        {
            // Get RTT
            if let Ok(rtt) = js_sys::Reflect::get(&value, &"currentRoundTripTime".into())
                && let Some(rtt_secs) = rtt.as_f64()
            {
                rtt_ms = Some(rtt_secs * 1000.0);
            }
            // Get local candidate ID to look up candidate type
            if let Ok(local_id) = js_sys::Reflect::get(&value, &"localCandidateId".into()) {
                local_candidate_id = local_id.as_string();
            }
        }
    });

    // Second pass: find the local candidate to get the connection type
    if let Some(ref candidate_id) = local_candidate_id {
        stats_map.for_each(&mut |value, key| {
            if key.as_string().as_deref() == Some(candidate_id.as_str())
                && let Ok(candidate_type) = js_sys::Reflect::get(&value, &"candidateType".into())
                && let Some(type_str) = candidate_type.as_string()
            {
                connection_type = match type_str.as_str() {
                    "host" => ConnectionType::Direct,
                    "srflx" | "prflx" => ConnectionType::Stun,
                    "relay" => ConnectionType::Turn,
                    _ => ConnectionType::Unknown,
                };
            }
        });
    }

    Some(PeerStats {
        peer_id,
        name,
        connection_type,
        rtt_ms,
    })
}

/// Update the peer stats display panel in the UI
pub fn update_peer_stats_display(stats: &[PeerStats]) {
    let Some(container) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("peer-stats-list"))
    else {
        return;
    };

    // Clear existing content
    container.set_inner_html("");

    if stats.is_empty() {
        let div = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("div").ok())
            .unwrap();
        let _ = div.set_attribute("class", "no-peers");
        div.set_text_content(Some("No peers connected"));
        let _ = container.append_child(&div);
        return;
    }

    for stat in stats {
        let row = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("div").ok())
            .unwrap();
        let _ = row.set_attribute("class", "peer-row");

        // Peer ID and name
        let id_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let _ = id_span.set_attribute("class", "peer-id");
        let id_text = match &stat.name {
            Some(name) => format!("#{} {}", stat.peer_id % 100, name),
            None => format!("#{}", stat.peer_id % 100),
        };
        id_span.set_text_content(Some(&id_text));

        // Connection type
        let type_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let (type_text, type_class) = match stat.connection_type {
            ConnectionType::Direct => ("Direct", "peer-type type-direct"),
            ConnectionType::Stun => ("STUN", "peer-type type-stun"),
            ConnectionType::Turn => ("TURN", "peer-type type-turn"),
            ConnectionType::Unknown => ("...", "peer-type"),
        };
        let _ = type_span.set_attribute("class", type_class);
        type_span.set_text_content(Some(type_text));

        // RTT
        let rtt_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let _ = rtt_span.set_attribute("class", "peer-rtt");
        let rtt_text = match stat.rtt_ms {
            Some(rtt) => format!("{:.0}ms", rtt),
            None => "-".to_string(),
        };
        rtt_span.set_text_content(Some(&rtt_text));

        let _ = row.append_child(&id_span);
        let _ = row.append_child(&type_span);
        let _ = row.append_child(&rtt_span);
        let _ = container.append_child(&row);
    }
}
