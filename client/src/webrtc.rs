use crate::player::{PlayerStateMessage, Team};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent, RtcIceCandidate,
    RtcIceCandidateInit, RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType,
    RtcSessionDescriptionInit, Url, WebSocket,
};

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

type PeerId = u64;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GameMessage {
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PeerInfo {
    id: PeerId,
    team: Team,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "welcome")]
    Welcome {
        #[serde(rename = "clientId")]
        client_id: PeerId,
        team: Team,
        peers: Vec<PeerInfo>,
    },
    #[serde(rename = "peer-joined")]
    PeerJoined {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
        team: Team,
    },
    #[serde(rename = "peer-left")]
    PeerLeft {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
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
    data_channel: Option<RtcDataChannel>,
    team: Team,
    pending_candidates: Vec<IceCandidate>,
}

#[derive(Clone)]
struct IceCandidate {
    candidate: String,
    sdp_mid: Option<String>,
    sdp_m_line_index: Option<u16>,
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct RtcState {
    local_id: Option<PeerId>,
    local_team: Option<Team>,
    peers: HashMap<PeerId, PeerConnection>,
    on_player_state: Option<Box<dyn Fn(PeerId, Vec3, f32)>>,
    on_team_assign: Option<Box<dyn Fn(Team)>>,
    on_peer_joined: Option<Box<dyn Fn(PeerId, Team)>>,
    on_peer_left: Option<Box<dyn Fn(PeerId)>>,
}

type StateRef = Rc<RefCell<RtcState>>;

pub struct WebRtcClient {
    state: StateRef,
}

impl WebRtcClient {
    pub fn new() -> Result<Self, JsValue> {
        let state: StateRef = Rc::new(RefCell::new(RtcState::default()));
        let server_url = signaling_server_url();
        log::info!("Connecting to signaling server: {}", server_url);
        let ws = WebSocket::new(&server_url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        // WebSocket open
        let ws_clone = ws.clone();
        set_callback(&ws, "onopen", move |_: JsValue| {
            log::info!("Connected to signaling server");
            update_status("Connected to server, joining game...");
            let _ = ws_clone.send_with_str(r#"{"type":"join"}"#);
        });

        // WebSocket message
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

        // WebSocket error/close
        set_callback(&ws, "onerror", |_: JsValue| {
            log::error!("WebSocket error");
            update_status("Connection error. Is the server running?");
        });
        set_callback(&ws, "onclose", |_: JsValue| {
            log::info!("WebSocket closed");
            update_status("Disconnected from server");
        });

        Ok(Self { state })
    }

    pub fn send_player_state(&self, position: Vec3, yaw: f32) {
        let state = self.state.borrow();
        if let Ok(json) = serde_json::to_string(&PlayerStateMessage::new(position, yaw)) {
            for peer in state.peers.values() {
                if let Some(ref dc) = peer.data_channel
                    && dc.ready_state() == web_sys::RtcDataChannelState::Open
                {
                    let _ = dc.send_with_str(&json);
                }
            }
        }
    }

    pub fn set_on_player_state<F: Fn(PeerId, Vec3, f32) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_player_state = Some(Box::new(callback));
    }

    pub fn set_on_team_assign<F: Fn(Team) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_team_assign = Some(Box::new(callback));
    }

    pub fn set_on_peer_joined<F: Fn(PeerId, Team) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_peer_joined = Some(Box::new(callback));
    }

    pub fn set_on_peer_left<F: Fn(PeerId) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_peer_left = Some(Box::new(callback));
    }
}

fn handle_signal(ws: &WebSocket, state: &StateRef, msg: ServerMessage) {
    let ws = ws.clone();
    let state = state.clone();

    wasm_bindgen_futures::spawn_local(async move {
        match msg {
            ServerMessage::Welcome {
                client_id,
                team,
                peers,
            } => {
                log::info!(
                    "Welcome! I am client {} on team {:?}, {} peers in game",
                    client_id,
                    team,
                    peers.len()
                );

                {
                    let mut s = state.borrow_mut();
                    s.local_id = Some(client_id);
                    s.local_team = Some(team);
                    if let Some(ref cb) = s.on_team_assign {
                        cb(team);
                    }
                }

                let status = format!(
                    "You are Team {} ({}). {} other player(s) in game.",
                    if team == Team::A { "A" } else { "B" },
                    if team == Team::A { "Blue" } else { "Red" },
                    peers.len()
                );
                update_status(&status);

                // Create peer connections and send offers to all existing peers
                for peer_info in peers {
                    if let Ok(pc) = create_peer_connection(&ws, &state, peer_info.id) {
                        let dc = pc.create_data_channel("game-sync");
                        setup_data_channel(&dc, &state, peer_info.id);

                        {
                            let mut s = state.borrow_mut();
                            s.peers.insert(
                                peer_info.id,
                                PeerConnection {
                                    pc: pc.clone(),
                                    data_channel: Some(dc),
                                    team: peer_info.team,
                                    pending_candidates: Vec::new(),
                                },
                            );
                            if let Some(ref cb) = s.on_peer_joined {
                                cb(peer_info.id, peer_info.team);
                            }
                        }

                        // Create and send offer
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
            ServerMessage::PeerJoined { peer_id, team } => {
                log::info!("Peer {} joined on team {:?}", peer_id, team);

                // Create a placeholder peer connection (will receive offer from them)
                if let Ok(pc) = create_peer_connection(&ws, &state, peer_id) {
                    let mut s = state.borrow_mut();
                    s.peers.insert(
                        peer_id,
                        PeerConnection {
                            pc,
                            data_channel: None,
                            team,
                            pending_candidates: Vec::new(),
                        },
                    );
                    if let Some(ref cb) = s.on_peer_joined {
                        cb(peer_id, team);
                    }
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
                    if let Some(ref cb) = s.on_peer_left {
                        cb(peer_id);
                    }
                }

                update_peer_count(&state);
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
                        let has = peer.pc.remote_description().is_some();
                        (has, Some(peer.pc.clone()))
                    } else {
                        (false, None)
                    }
                };

                if has_remote {
                    if let Some(pc) = pc {
                        add_ice_candidate(
                            &pc,
                            &IceCandidate {
                                candidate,
                                sdp_mid,
                                sdp_m_line_index,
                            },
                        )
                        .await;
                    }
                } else {
                    // Queue the candidate
                    let mut s = state.borrow_mut();
                    if let Some(peer) = s.peers.get_mut(&from_id) {
                        peer.pending_candidates.push(IceCandidate {
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

    // ICE candidate handler
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

    // ICE connection state change
    let state_clone = state.clone();
    let onice = Closure::wrap(Box::new(move |ev: JsValue| {
        if let Some(pc) = ev.dyn_ref::<RtcPeerConnection>() {
            match pc.ice_connection_state() {
                web_sys::RtcIceConnectionState::Connected => {
                    log::info!("Peer {} connected", peer_id);
                    update_peer_count(&state_clone);
                }
                web_sys::RtcIceConnectionState::Failed => {
                    log::warn!("Peer {} connection failed", peer_id);
                }
                web_sys::RtcIceConnectionState::Disconnected => {
                    log::warn!("Peer {} disconnected", peer_id);
                }
                _ => {}
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_oniceconnectionstatechange(Some(onice.as_ref().unchecked_ref()));
    onice.forget();

    // Data channel handler (for answerer side)
    let state_clone = state.clone();
    let ondc = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcDataChannelEvent = ev.unchecked_into();
        let dc = ev.channel();
        setup_data_channel(&dc, &state_clone, peer_id);
        let mut s = state_clone.borrow_mut();
        if let Some(peer) = s.peers.get_mut(&peer_id) {
            peer.data_channel = Some(dc);
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_ondatachannel(Some(ondc.as_ref().unchecked_ref()));
    ondc.forget();

    Ok(pc)
}

fn setup_data_channel(dc: &RtcDataChannel, state: &StateRef, peer_id: PeerId) {
    let state_clone = state.clone();
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("Data channel open with peer {}", peer_id);
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
        let Ok(msg) = serde_json::from_str::<GameMessage>(&data) else {
            return;
        };

        if msg.msg_type == "player_state"
            && let (Some(x), Some(y), Some(z), Some(yaw)) = (msg.x, msg.y, msg.z, msg.yaw)
        {
            let s = state_clone.borrow();
            if let Some(ref cb) = s.on_player_state {
                cb(peer_id, Vec3::new(x, y, z), yaw);
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
    onmsg.forget();
}

async fn apply_pending_candidates(pc: &RtcPeerConnection, state: &StateRef, peer_id: PeerId) {
    let pending: Vec<IceCandidate> = {
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

async fn add_ice_candidate(pc: &RtcPeerConnection, ice: &IceCandidate) {
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
    let msg = serde_json::json!({
        "type": "offer",
        "targetId": target_id,
        "sdp": sdp
    });
    let _ = ws.send_with_str(&msg.to_string());
}

fn send_answer(ws: &WebSocket, target_id: PeerId, sdp: &str) {
    let msg = serde_json::json!({
        "type": "answer",
        "targetId": target_id,
        "sdp": sdp
    });
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
            p.data_channel
                .as_ref()
                .is_some_and(|dc| dc.ready_state() == web_sys::RtcDataChannelState::Open)
        })
        .count();
    let total_count = s.peers.len();

    let status = if connected_count == total_count && total_count > 0 {
        format!(
            "Connected! {} player(s) in game.",
            total_count + 1 // +1 for self
        )
    } else if total_count > 0 {
        format!("Connecting... {}/{} peers", connected_count, total_count)
    } else {
        "Waiting for other players...".to_string()
    };
    update_status(&status);
}

thread_local! {
    pub static WEBRTC_CLIENT: RefCell<Option<WebRtcClient>> = const { RefCell::new(None) };
}

pub fn init_webrtc_client() {
    match WebRtcClient::new() {
        Ok(client) => WEBRTC_CLIENT.with(|c| *c.borrow_mut() = Some(client)),
        Err(e) => {
            log::error!("Failed to create WebRTC client: {:?}", e);
            update_status("Failed to connect to server");
        }
    }
}

pub fn send_player_state_to_peers(position: Vec3, yaw: f32) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.send_player_state(position, yaw);
        }
    });
}

pub fn set_player_state_callback<F: Fn(PeerId, Vec3, f32) + 'static>(callback: F) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.set_on_player_state(callback);
        }
    });
}

pub fn set_team_assign_callback<F: Fn(Team) + 'static>(callback: F) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.set_on_team_assign(callback);
        }
    });
}

pub fn set_peer_joined_callback<F: Fn(PeerId, Team) + 'static>(callback: F) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.set_on_peer_joined(callback);
        }
    });
}

pub fn set_peer_left_callback<F: Fn(PeerId) + 'static>(callback: F) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.set_on_peer_left(callback);
        }
    });
}
