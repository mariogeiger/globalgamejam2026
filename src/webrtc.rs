use crate::player::{PlayerStateMessage, Team};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent, RtcIceCandidate,
    RtcIceCandidateInit, RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType,
    RtcSessionDescriptionInit, WebSocket,
};

const SIGNALING_SERVER: &str = "wss://ggj26.cheapmo.ch";
const STUN_SERVERS: &[&str] = &[
    "stun:ggj26.cheapmo.ch:3478",
    "stun:stun.l.google.com:19302",
    "stun:stun1.l.google.com:19302",
];

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
struct SignalMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sdpMLineIndex")]
    sdp_m_line_index: Option<u16>,
}

// Consolidated state struct
#[derive(Default)]
struct RtcState {
    pc: Option<RtcPeerConnection>,
    data_channel: Option<RtcDataChannel>,
    local_team: Option<Team>,
    connected: bool,
    pending_candidates: Vec<SignalMessage>,
    on_player_state: Option<Box<dyn Fn(Vec3, f32)>>,
    on_team_assign: Option<Box<dyn Fn(Team)>>,
}

type StateRef = Rc<RefCell<RtcState>>;

pub struct WebRtcClient {
    state: StateRef,
}

impl WebRtcClient {
    pub fn new() -> Result<Self, JsValue> {
        let state: StateRef = Rc::new(RefCell::new(RtcState::default()));
        let ws = WebSocket::new(SIGNALING_SERVER)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        // WebSocket open
        let ws_clone = ws.clone();
        set_callback(&ws, "onopen", move |_: JsValue| {
            log::info!("Connected to signaling server");
            update_status("Connected to server, waiting for peer...");
            let _ = ws_clone.send_with_str(&serde_json::json!({"type": "join"}).to_string());
        });

        // WebSocket message
        let state_clone = state.clone();
        let ws_clone = ws.clone();
        set_callback(&ws, "onmessage", move |ev: MessageEvent| {
            if let Some(text) = ev.data().as_string() {
                if let Ok(msg) = serde_json::from_str::<SignalMessage>(&text) {
                    handle_signal(&ws_clone, &state_clone, msg);
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
        if let Some(ref dc) = state.data_channel {
            if dc.ready_state() == web_sys::RtcDataChannelState::Open {
                if let Ok(json) = serde_json::to_string(&PlayerStateMessage::new(position, yaw)) {
                    let _ = dc.send_with_str(&json);
                }
            }
        }
    }

    pub fn set_on_player_state<F: Fn(Vec3, f32) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_player_state = Some(Box::new(callback));
    }

    pub fn set_on_team_assign<F: Fn(Team) + 'static>(&self, callback: F) {
        self.state.borrow_mut().on_team_assign = Some(Box::new(callback));
    }
}

fn handle_signal(ws: &WebSocket, state: &StateRef, msg: SignalMessage) {
    let ws = ws.clone();
    let state = state.clone();

    wasm_bindgen_futures::spawn_local(async move {
        match msg.msg_type.as_str() {
            "waiting" => update_status("Waiting for another player..."),
            "waiting-for-offer" => {
                set_team(&state, Team::B);
                update_status("You are Team B (Red). Waiting for connection...");
            }
            "create-offer" => {
                set_team(&state, Team::A);
                update_status("You are Team A (Blue). Creating connection...");

                if let Ok(pc) = create_peer_connection(&ws, &state) {
                    let dc = pc.create_data_channel("game-sync");
                    setup_data_channel(&dc, &state);
                    state.borrow_mut().data_channel = Some(dc);
                    state.borrow_mut().pc = Some(pc.clone());

                    if let Ok(offer) = wasm_bindgen_futures::JsFuture::from(pc.create_offer()).await
                    {
                        let sdp = get_sdp(&offer);
                        let init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                        init.set_sdp(&sdp);
                        if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&init))
                            .await
                            .is_ok()
                        {
                            send_signal(&ws, "offer", Some(&sdp), None);
                        }
                    }
                }
            }
            "offer" => {
                if let Some(sdp) = msg.sdp {
                    if let Ok(pc) = create_peer_connection(&ws, &state) {
                        state.borrow_mut().pc = Some(pc.clone());

                        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                        desc.set_sdp(&sdp);
                        if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc))
                            .await
                            .is_ok()
                        {
                            apply_pending_candidates(&pc, &state).await;

                            if let Ok(answer) =
                                wasm_bindgen_futures::JsFuture::from(pc.create_answer()).await
                            {
                                let answer_sdp = get_sdp(&answer);
                                let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                                init.set_sdp(&answer_sdp);
                                if wasm_bindgen_futures::JsFuture::from(
                                    pc.set_local_description(&init),
                                )
                                .await
                                .is_ok()
                                {
                                    send_signal(&ws, "answer", Some(&answer_sdp), None);
                                }
                            }
                        }
                    }
                }
            }
            "answer" => {
                if let Some(sdp) = msg.sdp {
                    let state_ref = state.borrow();
                    if let Some(ref pc) = state_ref.pc {
                        let pc = pc.clone();
                        drop(state_ref);
                        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                        desc.set_sdp(&sdp);
                        if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc))
                            .await
                            .is_ok()
                        {
                            apply_pending_candidates(&pc, &state).await;
                        }
                    }
                }
            }
            "ice-candidate" => {
                if msg.candidate.is_some() {
                    let has_remote = state
                        .borrow()
                        .pc
                        .as_ref()
                        .map(|pc| pc.remote_description().is_some())
                        .unwrap_or(false);
                    if has_remote {
                        if let Some(ref pc) = state.borrow().pc {
                            add_ice_candidate(pc, &msg).await;
                        }
                    } else {
                        state.borrow_mut().pending_candidates.push(msg);
                    }
                }
            }
            "peer-disconnected" => {
                update_status("Peer disconnected. Refresh to reconnect.");
                state.borrow_mut().connected = false;
            }
            _ => {}
        }
    });
}

fn set_team(state: &StateRef, team: Team) {
    let mut s = state.borrow_mut();
    s.local_team = Some(team);
    if let Some(ref cb) = s.on_team_assign {
        cb(team);
    }
}

fn create_peer_connection(ws: &WebSocket, state: &StateRef) -> Result<RtcPeerConnection, JsValue> {
    let config = RtcConfiguration::new();
    let ice_servers = js_sys::Array::new();
    let urls = js_sys::Array::new();
    for url in STUN_SERVERS {
        urls.push(&(*url).into());
    }
    let server = js_sys::Object::new();
    js_sys::Reflect::set(&server, &"urls".into(), &urls)?;
    ice_servers.push(&server);
    config.set_ice_servers(&ice_servers);

    let pc = RtcPeerConnection::new_with_configuration(&config)?;

    // ICE candidate
    let ws_clone = ws.clone();
    let onicecandidate = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
        if let Some(c) = ev.candidate() {
            let msg = serde_json::json!({
                "type": "ice-candidate",
                "candidate": c.candidate(),
                "sdpMid": c.sdp_mid(),
                "sdpMLineIndex": c.sdp_m_line_index()
            });
            let _ = ws_clone.send_with_str(&msg.to_string());
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_onicecandidate(Some(onicecandidate.as_ref().unchecked_ref()));
    onicecandidate.forget();

    // ICE state change
    let state_clone = state.clone();
    let onice = Closure::wrap(Box::new(move |ev: JsValue| {
        if let Some(pc) = ev.dyn_ref::<RtcPeerConnection>() {
            match pc.ice_connection_state() {
                web_sys::RtcIceConnectionState::Connected => {
                    state_clone.borrow_mut().connected = true
                }
                web_sys::RtcIceConnectionState::Failed => update_status("Connection failed."),
                web_sys::RtcIceConnectionState::Disconnected => update_status("Connection lost."),
                _ => {}
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_oniceconnectionstatechange(Some(onice.as_ref().unchecked_ref()));
    onice.forget();

    // Data channel (for answerer)
    let state_clone = state.clone();
    let ondc = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcDataChannelEvent = ev.unchecked_into();
        let dc = ev.channel();
        setup_data_channel(&dc, &state_clone);
        state_clone.borrow_mut().data_channel = Some(dc);
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_ondatachannel(Some(ondc.as_ref().unchecked_ref()));
    ondc.forget();

    Ok(pc)
}

fn setup_data_channel(dc: &RtcDataChannel, state: &StateRef) {
    let state_clone = state.clone();
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        state_clone.borrow_mut().connected = true;
        update_status("Connected! Both players ready.");
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let state_clone = state.clone();
    let onmsg = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        if let Some(data) = ev.data().as_string() {
            if let Ok(msg) = serde_json::from_str::<GameMessage>(&data) {
                if msg.msg_type == "player_state" {
                    if let (Some(x), Some(y), Some(z), Some(yaw)) = (msg.x, msg.y, msg.z, msg.yaw) {
                        if let Some(ref cb) = state_clone.borrow().on_player_state {
                            cb(Vec3::new(x, y, z), yaw);
                        }
                    }
                }
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
    onmsg.forget();
}

async fn apply_pending_candidates(pc: &RtcPeerConnection, state: &StateRef) {
    let pending: Vec<_> = state.borrow_mut().pending_candidates.drain(..).collect();
    for msg in pending {
        add_ice_candidate(pc, &msg).await;
    }
}

async fn add_ice_candidate(pc: &RtcPeerConnection, msg: &SignalMessage) {
    if let Some(ref candidate) = msg.candidate {
        let init = RtcIceCandidateInit::new(candidate);
        if let Some(ref mid) = msg.sdp_mid {
            init.set_sdp_mid(Some(mid));
        }
        if let Some(idx) = msg.sdp_m_line_index {
            init.set_sdp_m_line_index(Some(idx));
        }
        if let Ok(ice) = RtcIceCandidate::new(&init) {
            let _ = wasm_bindgen_futures::JsFuture::from(
                pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&ice)),
            )
            .await;
        }
    }
}

fn get_sdp(js: &JsValue) -> String {
    js_sys::Reflect::get(js, &"sdp".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

fn send_signal(ws: &WebSocket, msg_type: &str, sdp: Option<&str>, _: Option<()>) {
    let mut msg = serde_json::json!({"type": msg_type});
    if let Some(s) = sdp {
        msg["sdp"] = serde_json::Value::String(s.to_string());
    }
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

pub fn send_player_state_to_peer(position: Vec3, yaw: f32) {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = *c.borrow() {
            client.send_player_state(position, yaw);
        }
    });
}

pub fn set_player_state_callback<F: Fn(Vec3, f32) + 'static>(callback: F) {
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
