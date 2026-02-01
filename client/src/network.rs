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

#[derive(Default)]
struct RtcState {
    local_id: Option<PeerId>,
    peers: HashMap<PeerId, PeerConnection>,
    pending_events: Vec<NetworkEvent>,
}

type StateRef = Rc<RefCell<RtcState>>;

pub struct NetworkClient {
    state: StateRef,
    ws: WebSocket,
}

impl NetworkClient {
    pub fn new() -> Result<Self, JsValue> {
        let state: StateRef = Rc::new(RefCell::new(RtcState::default()));
        let server_url = signaling_server_url();
        log::info!("Connecting to signaling server: {}", server_url);
        let ws = WebSocket::new(&server_url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let ws_clone = ws.clone();
        set_callback(&ws, "onopen", move |_: JsValue| {
            log::info!("Connected to signaling server");
            update_status("Connected to server, joining game...");
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
            update_status("Connection error. Is the server running?");
        });
        set_callback(&ws, "onclose", |_: JsValue| {
            log::info!("WebSocket closed");
            update_status("Disconnected from server");
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
            for peer in state.peers.values() {
                if let Some(ref dc) = peer.state_channel
                    && dc.ready_state() == web_sys::RtcDataChannelState::Open
                {
                    let _ = dc.send_with_str(&json);
                }
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

fn stun_servers() -> Vec<String> {
    let mut servers = vec![
        "stun:stun.l.google.com:19302".to_string(),
        "stun:stun1.l.google.com:19302".to_string(),
    ];
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href()
            && let Ok(url) = Url::new(&href)
        {
            let hostname = url.hostname();
            if !hostname.is_empty() && hostname != "localhost" {
                servers.insert(0, format!("stun:{}:3478", hostname));
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

                let status = if peers.is_empty() {
                    "Connected! Waiting for other players...".to_string()
                } else if peers.len() == 1 {
                    "Connected! 1 other player in game.".to_string()
                } else {
                    format!("Connected! {} other players in game.", peers.len())
                };
                update_status(&status);

                for peer_info in peers {
                    if let Ok(pc) = create_peer_connection(&ws, &state, peer_info.id) {
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

                if let Ok(pc) = create_peer_connection(&ws, &state, peer_id) {
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
                update_peer_count(&state);
            }
            ServerMessage::PeerLeft { peer_id } => {
                log::info!("Peer {} left", peer_id);
                {
                    let mut s = state.borrow_mut();
                    if let Some(peer) = s.peers.remove(&peer_id) {
                        peer.pc.close();
                    }
                    s.pending_events
                        .push(NetworkEvent::PeerLeft { id: peer_id });
                }
                update_peer_count(&state);
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
                                send_answer(&ws, from_id, &answer_sdp);
                            }
                        }
                    }
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
                        apply_pending_candidates(&pc, &state, from_id).await;
                    }
                }
            }
            ServerMessage::IceCandidate {
                from_id,
                candidate,
                sdp_mid,
                sdp_m_line_index,
            } => {
                let (has_remote, pc) = {
                    let s = state.borrow();
                    if let Some(peer) = s.peers.get(&from_id) {
                        (
                            peer.pc.remote_description().is_some(),
                            Some(peer.pc.clone()),
                        )
                    } else {
                        (false, None)
                    }
                };

                if has_remote {
                    if let Some(pc) = pc {
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

fn create_peer_connection(
    ws: &WebSocket,
    state: &StateRef,
    peer_id: PeerId,
) -> Result<RtcPeerConnection, JsValue> {
    let config = RtcConfiguration::new();
    let ice_servers = js_sys::Array::new();
    let urls = js_sys::Array::new();
    for url in stun_servers() {
        urls.push(&url.into());
    }
    let server = js_sys::Object::new();
    js_sys::Reflect::set(&server, &"urls".into(), &urls)?;
    ice_servers.push(&server);
    config.set_ice_servers(&ice_servers);

    let pc = RtcPeerConnection::new_with_configuration(&config)?;

    let ws_clone = ws.clone();
    let onicecandidate = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
        if let Some(c) = ev.candidate() {
            let msg = serde_json::json!({
                "type": "ice-candidate",
                "targetId": peer_id,
                "candidate": c.candidate(),
                "sdpMid": c.sdp_mid(),
                "sdpMLineIndex": c.sdp_m_line_index()
            });
            let _ = ws_clone.send_with_str(&msg.to_string());
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_onicecandidate(Some(onicecandidate.as_ref().unchecked_ref()));
    onicecandidate.forget();

    let state_clone = state.clone();
    let onice = Closure::wrap(Box::new(move |ev: JsValue| {
        if let Some(pc) = ev.dyn_ref::<RtcPeerConnection>() {
            match pc.ice_connection_state() {
                web_sys::RtcIceConnectionState::Connected => {
                    log::info!("Peer {} connected", peer_id);
                    update_peer_count(&state_clone);
                }
                web_sys::RtcIceConnectionState::Failed => {
                    log::warn!("Peer {} connection failed", peer_id)
                }
                web_sys::RtcIceConnectionState::Disconnected => {
                    log::warn!("Peer {} disconnected", peer_id)
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

        match label.as_str() {
            "state" => {
                setup_state_channel(&dc, &state_clone, peer_id);
                let mut s = state_clone.borrow_mut();
                if let Some(peer) = s.peers.get_mut(&peer_id) {
                    peer.state_channel = Some(dc);
                }
            }
            "events" => {
                setup_events_channel(&dc, &state_clone, peer_id);
                let mut s = state_clone.borrow_mut();
                if let Some(peer) = s.peers.get_mut(&peer_id) {
                    peer.events_channel = Some(dc);
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
    let state_clone = state.clone();
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("State channel open with peer {}", peer_id);
        update_peer_count(&state_clone);
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let state_clone = state.clone();
    let onmsg = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        let Some(data) = ev.data().as_string() else {
            return;
        };
        let Ok(msg) = serde_json::from_str::<PlayerStateMessage>(&data) else {
            return;
        };

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

/// Set up the reliable "events" channel for game events (kills)
fn setup_events_channel(dc: &RtcDataChannel, state: &StateRef, peer_id: PeerId) {
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("Events channel open with peer {}", peer_id);
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

fn update_status(status: &str) {
    if let Some(elem) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("connection-status"))
    {
        elem.set_text_content(Some(status));
    }
}

fn update_peer_count(state: &StateRef) {
    let s = state.borrow();
    let connected_count = s
        .peers
        .values()
        .filter(|p| {
            // Consider connected if state channel is open (primary channel for gameplay)
            p.state_channel
                .as_ref()
                .is_some_and(|dc| dc.ready_state() == web_sys::RtcDataChannelState::Open)
        })
        .count();
    let total_peers = s.peers.len();

    let status = if total_peers == 0 {
        "Waiting for other players...".to_string()
    } else if connected_count < total_peers {
        format!("Connecting... {}/{} peers", connected_count, total_peers)
    } else {
        // All peers connected, total_peers + 1 = total players including self
        let total_players = total_peers + 1;
        format!("Connected! {} players in game.", total_players)
    };
    update_status(&status);
}
