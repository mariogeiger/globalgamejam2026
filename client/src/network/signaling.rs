//! WebSocket signaling client.
//!
//! Handles communication with the signaling server for WebRTC connection establishment.
//! This includes sending/receiving SDP offers, answers, and ICE candidates.

use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

use super::ice::signaling_server_url;
use super::protocol::{PeerId, SignalCommand, SignalMessage};
use super::queue::EventQueue;
use super::ui::{NetLogLevel, net_log};

/// Events from the signaling server.
#[derive(Debug, Clone)]
pub enum SignalingEvent {
    /// Connection to signaling server opened.
    Connected,
    /// Connection to signaling server closed.
    Disconnected,
    /// Error on signaling connection.
    Error,
    /// Received a message from the signaling server.
    Message(SignalMessage),
}

/// WebSocket signaling client.
pub struct SignalingClient {
    ws: WebSocket,
    incoming: Rc<EventQueue<SignalingEvent>>,
}

impl SignalingClient {
    /// Connect to the signaling server.
    pub fn connect() -> Result<Self, JsValue> {
        let server_url = signaling_server_url();
        log::info!("Connecting to signaling server: {}", server_url);

        let ws = WebSocket::new(&server_url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let incoming = Rc::new(EventQueue::new());

        // Set up onopen handler
        let ws_clone = ws.clone();
        let incoming_clone = incoming.clone();
        let onopen = Closure::wrap(Box::new(move |_: JsValue| {
            log::info!("Connected to signaling server");
            net_log(NetLogLevel::Success, "Connected to signaling server");
            incoming_clone.push(SignalingEvent::Connected);

            // Send join message
            let cmd = SignalCommand::Join;
            if let Ok(json) = serde_json::to_string(&cmd) {
                let _ = ws_clone.send_with_str(&json);
            }
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // Set up onmessage handler
        let incoming_clone = incoming.clone();
        let onmessage = Closure::wrap(Box::new(move |ev: MessageEvent| {
            if let Some(text) = ev.data().as_string() {
                match serde_json::from_str::<SignalMessage>(&text) {
                    Ok(msg) => {
                        incoming_clone.push(SignalingEvent::Message(msg));
                    }
                    Err(e) => {
                        log::warn!("Failed to parse server message: {} ({})", text, e);
                    }
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        // Set up onerror handler
        let incoming_clone = incoming.clone();
        let onerror = Closure::wrap(Box::new(move |_: JsValue| {
            log::error!("WebSocket error");
            net_log(NetLogLevel::Error, "WebSocket error - server unreachable?");
            incoming_clone.push(SignalingEvent::Error);
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        // Set up onclose handler
        let incoming_clone = incoming.clone();
        let onclose = Closure::wrap(Box::new(move |_: JsValue| {
            log::info!("WebSocket closed");
            net_log(NetLogLevel::Warning, "Disconnected from server");
            incoming_clone.push(SignalingEvent::Disconnected);
        }) as Box<dyn FnMut(JsValue)>);
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();

        Ok(Self { ws, incoming })
    }

    /// Poll for incoming signaling events.
    pub fn poll_events(&self) -> Vec<SignalingEvent> {
        self.incoming.drain()
    }

    /// Send an SDP offer to a peer.
    pub fn send_offer(&self, target_id: PeerId, sdp: &str) {
        let cmd = SignalCommand::Offer {
            target_id,
            sdp: sdp.to_string(),
        };
        self.send_command(&cmd);
    }

    /// Send an SDP answer to a peer.
    pub fn send_answer(&self, target_id: PeerId, sdp: &str) {
        let cmd = SignalCommand::Answer {
            target_id,
            sdp: sdp.to_string(),
        };
        self.send_command(&cmd);
    }

    /// Send an ICE candidate to a peer.
    pub fn send_ice_candidate(
        &self,
        target_id: PeerId,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
    ) {
        let cmd = SignalCommand::IceCandidate {
            target_id,
            candidate,
            sdp_mid,
            sdp_m_line_index,
        };
        self.send_command(&cmd);
    }

    /// Notify server that we died.
    pub fn send_player_died(&self) {
        let cmd = SignalCommand::PlayerDied;
        self.send_command(&cmd);
    }

    /// Send leave message and close connection.
    pub fn disconnect(&self) {
        let cmd = SignalCommand::Leave;
        self.send_command(&cmd);
        let _ = self.ws.close();
    }

    /// Send a command to the signaling server.
    fn send_command(&self, cmd: &SignalCommand) {
        if let Ok(json) = serde_json::to_string(cmd) {
            let _ = self.ws.send_with_str(&json);
        }
    }
}
