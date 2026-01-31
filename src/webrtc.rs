use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    MessageEvent, RtcDataChannel, RtcDataChannelEvent, RtcIceCandidate,
    RtcIceCandidateInit, RtcPeerConnection, RtcPeerConnectionIceEvent,
    RtcSessionDescriptionInit, RtcSdpType, WebSocket, RtcConfiguration,
};
use serde::{Deserialize, Serialize};
use glam::Vec3;
use crate::network_player::{Team, RemotePlayer, PlayerStateMessage};

const SIGNALING_SERVER: &str = "wss://ggj26.cheapmo.ch";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
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

pub struct WebRtcClient {
    #[allow(dead_code)]
    ws: WebSocket,
    #[allow(dead_code)]
    pc: Rc<RefCell<Option<RtcPeerConnection>>>,
    data_channel: Rc<RefCell<Option<RtcDataChannel>>>,
    on_player_state: Rc<RefCell<Option<Box<dyn Fn(Vec3, f32)>>>>,
    on_team_assign: Rc<RefCell<Option<Box<dyn Fn(Team)>>>>,
    local_team: Rc<RefCell<Option<Team>>>,
    #[allow(dead_code)]
    connected: Rc<RefCell<bool>>,
    #[allow(dead_code)]
    pending_candidates: Rc<RefCell<Vec<SignalMessage>>>,
}

impl WebRtcClient {
    pub fn new() -> Result<Self, JsValue> {
        let ws = WebSocket::new(SIGNALING_SERVER)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        
        let pc: Rc<RefCell<Option<RtcPeerConnection>>> = Rc::new(RefCell::new(None));
        let data_channel: Rc<RefCell<Option<RtcDataChannel>>> = Rc::new(RefCell::new(None));
        let on_player_state: Rc<RefCell<Option<Box<dyn Fn(Vec3, f32)>>>> = Rc::new(RefCell::new(None));
        let on_team_assign: Rc<RefCell<Option<Box<dyn Fn(Team)>>>> = Rc::new(RefCell::new(None));
        let local_team: Rc<RefCell<Option<Team>>> = Rc::new(RefCell::new(None));
        let connected = Rc::new(RefCell::new(false));
        let pending_candidates: Rc<RefCell<Vec<SignalMessage>>> = Rc::new(RefCell::new(Vec::new()));
        
        // WebSocket open handler
        let ws_clone = ws.clone();
        let onopen = Closure::wrap(Box::new(move |_: JsValue| {
            log::info!("Connected to signaling server");
            update_status("Connected to server, waiting for peer...");
            
            // Send join message
            let msg = serde_json::json!({ "type": "join" });
            let _ = ws_clone.send_with_str(&msg.to_string());
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();
        
        // WebSocket message handler
        let ws_for_msg = ws.clone();
        let pc_for_msg = pc.clone();
        let dc_for_msg = data_channel.clone();
        let on_player_state_for_msg = on_player_state.clone();
        let on_team_assign_for_msg = on_team_assign.clone();
        let local_team_for_msg = local_team.clone();
        let connected_for_msg = connected.clone();
        let pending_for_msg = pending_candidates.clone();
        
        let onmessage = Closure::wrap(Box::new(move |ev: MessageEvent| {
            if let Some(text) = ev.data().as_string() {
                if let Ok(msg) = serde_json::from_str::<SignalMessage>(&text) {
                    handle_signal_message(
                        &ws_for_msg,
                        &pc_for_msg,
                        &dc_for_msg,
                        &on_player_state_for_msg,
                        &on_team_assign_for_msg,
                        &local_team_for_msg,
                        &connected_for_msg,
                        &pending_for_msg,
                        msg,
                    );
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
        
        // WebSocket error handler
        let onerror = Closure::wrap(Box::new(move |e: JsValue| {
            log::error!("WebSocket error: {:?}", e);
            update_status("Connection error. Is the server running?");
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();
        
        // WebSocket close handler
        let onclose = Closure::wrap(Box::new(move |_: JsValue| {
            log::info!("WebSocket closed");
            update_status("Disconnected from server");
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();
        
        Ok(Self {
            ws,
            pc,
            data_channel,
            on_player_state,
            on_team_assign,
            local_team,
            connected,
            pending_candidates,
        })
    }
    
    pub fn send_player_state(&self, position: Vec3, yaw: f32) {
        if let Some(ref dc) = *self.data_channel.borrow() {
            if dc.ready_state() == web_sys::RtcDataChannelState::Open {
                let msg = PlayerStateMessage::new(position, yaw);
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = dc.send_with_str(&json);
                }
            }
        }
    }
    
    pub fn set_on_player_state<F: Fn(Vec3, f32) + 'static>(&self, callback: F) {
        *self.on_player_state.borrow_mut() = Some(Box::new(callback));
    }
    
    pub fn set_on_team_assign<F: Fn(Team) + 'static>(&self, callback: F) {
        *self.on_team_assign.borrow_mut() = Some(Box::new(callback));
    }
    
    pub fn get_local_team(&self) -> Option<Team> {
        *self.local_team.borrow()
    }
}

fn handle_signal_message(
    ws: &WebSocket,
    pc_cell: &Rc<RefCell<Option<RtcPeerConnection>>>,
    dc_cell: &Rc<RefCell<Option<RtcDataChannel>>>,
    on_player_state: &Rc<RefCell<Option<Box<dyn Fn(Vec3, f32)>>>>,
    on_team_assign: &Rc<RefCell<Option<Box<dyn Fn(Team)>>>>,
    local_team: &Rc<RefCell<Option<Team>>>,
    connected: &Rc<RefCell<bool>>,
    pending_candidates: &Rc<RefCell<Vec<SignalMessage>>>,
    msg: SignalMessage,
) {
    let ws = ws.clone();
    let pc_cell = pc_cell.clone();
    let dc_cell = dc_cell.clone();
    let on_player_state = on_player_state.clone();
    let on_team_assign = on_team_assign.clone();
    let local_team = local_team.clone();
    let connected = connected.clone();
    let pending_candidates = pending_candidates.clone();
    
    wasm_bindgen_futures::spawn_local(async move {
        match msg.msg_type.as_str() {
            "waiting" => {
                update_status("Waiting for another player...");
            }
            "waiting-for-offer" => {
                // Second player joins - they are Team B
                *local_team.borrow_mut() = Some(Team::B);
                if let Some(ref callback) = *on_team_assign.borrow() {
                    callback(Team::B);
                }
                update_status("You are Team B (Red). Waiting for connection...");
            }
            "create-offer" => {
                log::info!("Creating offer...");
                // First player (offerer) is Team A
                *local_team.borrow_mut() = Some(Team::A);
                if let Some(ref callback) = *on_team_assign.borrow() {
                    callback(Team::A);
                }
                update_status("You are Team A (Blue). Creating connection...");
                
                // Create peer connection with STUN
                if let Ok(pc) = create_peer_connection(&ws, &dc_cell, &on_player_state, &connected) {
                    // Store it immediately so ICE candidates can be added
                    *pc_cell.borrow_mut() = Some(pc.clone());
                    
                    // Create data channel
                    let dc = pc.create_data_channel("game-sync");
                    setup_data_channel(&dc, &on_player_state, &connected);
                    *dc_cell.borrow_mut() = Some(dc);
                    
                    // Create offer
                    match wasm_bindgen_futures::JsFuture::from(pc.create_offer()).await {
                        Ok(offer) => {
                            let offer_sdp = js_sys::Reflect::get(&offer, &JsValue::from_str("sdp"))
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();
                            
                            let offer_init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                            offer_init.set_sdp(&offer_sdp);
                            
                            if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&offer_init)).await.is_ok() {
                                log::info!("Offer created and set");
                                let msg = serde_json::json!({
                                    "type": "offer",
                                    "sdp": offer_sdp
                                });
                                let _ = ws.send_with_str(&msg.to_string());
                            }
                        }
                        Err(e) => log::error!("Failed to create offer: {:?}", e),
                    }
                }
            }
            "offer" => {
                log::info!("Received offer");
                update_status("Received offer, creating answer...");
                
                if let Some(sdp) = msg.sdp {
                    // Create peer connection with STUN
                    if let Ok(pc) = create_peer_connection(&ws, &dc_cell, &on_player_state, &connected) {
                        // Store it immediately
                        *pc_cell.borrow_mut() = Some(pc.clone());
                        
                        // Set remote description
                        let remote_desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                        remote_desc.set_sdp(&sdp);
                        
                        if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&remote_desc)).await.is_ok() {
                            log::info!("Remote description set");
                            
                            // Add any pending ICE candidates
                            for pending in pending_candidates.borrow().iter() {
                                if let Some(ref candidate_str) = pending.candidate {
                                    add_ice_candidate(&pc, candidate_str, &pending.sdp_mid, pending.sdp_m_line_index).await;
                                }
                            }
                            pending_candidates.borrow_mut().clear();
                            
                            // Create answer
                            if let Ok(answer) = wasm_bindgen_futures::JsFuture::from(pc.create_answer()).await {
                                let answer_sdp = js_sys::Reflect::get(&answer, &JsValue::from_str("sdp"))
                                    .ok()
                                    .and_then(|v| v.as_string())
                                    .unwrap_or_default();
                                
                                let answer_init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                                answer_init.set_sdp(&answer_sdp);
                                
                                if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&answer_init)).await.is_ok() {
                                    log::info!("Answer created and set");
                                    update_status("Answer sent, connecting...");
                                    let msg = serde_json::json!({
                                        "type": "answer",
                                        "sdp": answer_sdp
                                    });
                                    let _ = ws.send_with_str(&msg.to_string());
                                }
                            }
                        } else {
                            log::error!("Failed to set remote description");
                        }
                    }
                }
            }
            "answer" => {
                log::info!("Received answer");
                update_status("Answer received, connecting...");
                
                if let Some(sdp) = msg.sdp {
                    if let Some(ref pc) = *pc_cell.borrow() {
                        let remote_desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                        remote_desc.set_sdp(&sdp);
                        
                        if wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&remote_desc)).await.is_ok() {
                            log::info!("Remote answer set");
                            
                            // Add any pending ICE candidates
                            for pending in pending_candidates.borrow().iter() {
                                if let Some(ref candidate_str) = pending.candidate {
                                    add_ice_candidate(pc, candidate_str, &pending.sdp_mid, pending.sdp_m_line_index).await;
                                }
                            }
                            pending_candidates.borrow_mut().clear();
                        }
                    }
                }
            }
            "ice-candidate" => {
                if let Some(ref candidate_str) = msg.candidate {
                    if let Some(ref pc) = *pc_cell.borrow() {
                        // Check if we have a remote description set
                        if pc.remote_description().is_some() {
                            add_ice_candidate(pc, candidate_str, &msg.sdp_mid, msg.sdp_m_line_index).await;
                        } else {
                            // Buffer the candidate for later
                            log::info!("Buffering ICE candidate");
                            pending_candidates.borrow_mut().push(msg);
                        }
                    } else {
                        // Buffer the candidate for later
                        log::info!("Buffering ICE candidate (no PC yet)");
                        pending_candidates.borrow_mut().push(msg);
                    }
                }
            }
            "peer-disconnected" => {
                update_status("Peer disconnected. Refresh to reconnect.");
                *connected.borrow_mut() = false;
            }
            _ => {}
        }
    });
}

async fn add_ice_candidate(pc: &RtcPeerConnection, candidate: &str, sdp_mid: &Option<String>, sdp_m_line_index: Option<u16>) {
    let init = RtcIceCandidateInit::new(candidate);
    if let Some(ref mid) = sdp_mid {
        init.set_sdp_mid(Some(mid));
    }
    if let Some(idx) = sdp_m_line_index {
        init.set_sdp_m_line_index(Some(idx));
    }
    
    if let Ok(ice_candidate) = RtcIceCandidate::new(&init) {
        match wasm_bindgen_futures::JsFuture::from(
            pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&ice_candidate))
        ).await {
            Ok(_) => log::info!("Added ICE candidate"),
            Err(e) => log::error!("Failed to add ICE candidate: {:?}", e),
        }
    }
}

fn create_peer_connection(
    ws: &WebSocket,
    dc_cell: &Rc<RefCell<Option<RtcDataChannel>>>,
    on_player_state: &Rc<RefCell<Option<Box<dyn Fn(Vec3, f32)>>>>,
    connected: &Rc<RefCell<bool>>,
) -> Result<RtcPeerConnection, JsValue> {
    // Configure with multiple STUN servers for NAT traversal
    // TURN servers require credentials - free ones are unreliable
    let config = RtcConfiguration::new();
    let ice_servers = js_sys::Array::new();
    
    // Multiple STUN servers for better connectivity
    let stun_urls = js_sys::Array::new();
    stun_urls.push(&"stun:stun.l.google.com:19302".into());
    stun_urls.push(&"stun:stun1.l.google.com:19302".into());
    stun_urls.push(&"stun:stun2.l.google.com:19302".into());
    stun_urls.push(&"stun:stun3.l.google.com:19302".into());
    stun_urls.push(&"stun:stun4.l.google.com:19302".into());
    
    let stun_server = js_sys::Object::new();
    js_sys::Reflect::set(&stun_server, &"urls".into(), &stun_urls)?;
    ice_servers.push(&stun_server);
    
    config.set_ice_servers(&ice_servers);
    
    let pc = RtcPeerConnection::new_with_configuration(&config)?;
    
    // ICE candidate handler
    let ws_clone = ws.clone();
    let onicecandidate = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
        if let Some(candidate) = ev.candidate() {
            log::info!("Sending ICE candidate");
            let msg = serde_json::json!({
                "type": "ice-candidate",
                "candidate": candidate.candidate(),
                "sdpMid": candidate.sdp_mid(),
                "sdpMLineIndex": candidate.sdp_m_line_index()
            });
            let _ = ws_clone.send_with_str(&msg.to_string());
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_onicecandidate(Some(onicecandidate.as_ref().unchecked_ref()));
    onicecandidate.forget();
    
    // ICE connection state change handler
    let connected_for_ice = connected.clone();
    let oniceconnectionstatechange = Closure::wrap(Box::new(move |ev: JsValue| {
        if let Some(pc) = ev.dyn_ref::<RtcPeerConnection>() {
            let state = pc.ice_connection_state();
            log::info!("ICE connection state: {:?}", state);
            match state {
                web_sys::RtcIceConnectionState::Connected => {
                    *connected_for_ice.borrow_mut() = true;
                }
                web_sys::RtcIceConnectionState::Failed => {
                    update_status("Connection failed. Try refreshing.");
                }
                web_sys::RtcIceConnectionState::Disconnected => {
                    update_status("Connection lost.");
                }
                _ => {}
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_oniceconnectionstatechange(Some(oniceconnectionstatechange.as_ref().unchecked_ref()));
    oniceconnectionstatechange.forget();
    
    // Data channel handler (for answerer)
    let dc_cell_clone = dc_cell.clone();
    let on_player_state_clone = on_player_state.clone();
    let connected_clone = connected.clone();
    let ondatachannel = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: RtcDataChannelEvent = ev.unchecked_into();
        let dc = ev.channel();
        log::info!("Received data channel: {}", dc.label());
        
        setup_data_channel(&dc, &on_player_state_clone, &connected_clone);
        *dc_cell_clone.borrow_mut() = Some(dc);
    }) as Box<dyn FnMut(JsValue)>);
    pc.set_ondatachannel(Some(ondatachannel.as_ref().unchecked_ref()));
    ondatachannel.forget();
    
    Ok(pc)
}

fn setup_data_channel(
    dc: &RtcDataChannel,
    on_player_state: &Rc<RefCell<Option<Box<dyn Fn(Vec3, f32)>>>>,
    connected: &Rc<RefCell<bool>>,
) {
    let connected_clone = connected.clone();
    let onopen = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("Data channel opened!");
        *connected_clone.borrow_mut() = true;
        update_status("Connected! Both players ready.");
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();
    
    let onclose = Closure::wrap(Box::new(move |_: JsValue| {
        log::info!("Data channel closed");
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
    
    let on_player_state_clone = on_player_state.clone();
    let onmessage = Closure::wrap(Box::new(move |ev: JsValue| {
        let ev: MessageEvent = ev.unchecked_into();
        if let Some(data) = ev.data().as_string() {
            if let Ok(msg) = serde_json::from_str::<GameMessage>(&data) {
                if msg.msg_type == "player_state" {
                    if let (Some(x), Some(y), Some(z), Some(yaw)) = (msg.x, msg.y, msg.z, msg.yaw) {
                        if let Some(ref callback) = *on_player_state_clone.borrow() {
                            callback(Vec3::new(x, y, z), yaw);
                        }
                    }
                }
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    dc.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();
}

fn update_status(status: &str) {
    if let Some(window) = web_sys::window() {
        if let Some(document) = window.document() {
            if let Some(elem) = document.get_element_by_id("connection-status") {
                elem.set_text_content(Some(status));
            }
        }
    }
}

thread_local! {
    pub static WEBRTC_CLIENT: RefCell<Option<WebRtcClient>> = const { RefCell::new(None) };
}

pub fn init_webrtc_client() {
    match WebRtcClient::new() {
        Ok(client) => {
            WEBRTC_CLIENT.with(|c| {
                *c.borrow_mut() = Some(client);
            });
        }
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

pub fn get_local_team() -> Option<Team> {
    WEBRTC_CLIENT.with(|c| {
        if let Some(ref client) = c.borrow().as_ref() {
            client.get_local_team()
        } else {
            None
        }
    })
}
