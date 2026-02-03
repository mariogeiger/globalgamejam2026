//! WebRTC transport layer.
//!
//! Provides abstractions over WebRTC peer connections and data channels.
//! This layer knows nothing about game messages - it just sends and receives strings.

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    MessageEvent, RtcConfiguration, RtcDataChannel, RtcDataChannelEvent, RtcDataChannelInit,
    RtcPeerConnection, RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit,
};

use super::ice::{fetch_turn_servers, to_js_ice_servers};
use super::protocol::{ChannelKind, PeerId};
use super::queue::EventQueue;
use super::ui::{NetLogLevel, net_log};

// Thread-local storage for channels received via ondatachannel callback.
// This allows immediate storage like the old design, avoiding event queue timing issues.
thread_local! {
    pub static RECEIVED_CHANNELS: RefCell<Vec<ReceivedChannel>> = const { RefCell::new(Vec::new()) };
}

/// A channel received from a remote peer that needs to be stored.
pub struct ReceivedChannel {
    pub peer_id: PeerId,
    pub kind: ChannelKind,
    pub channel: RtcDataChannel,
}

/// Events from a WebRTC peer connection.
#[derive(Debug, Clone)]
pub enum PeerEvent {
    /// Data channel is now open and ready.
    ChannelOpened(ChannelKind),
    /// Received a message on a channel.
    Message { channel: ChannelKind, data: String },
    /// ICE connection state changed.
    IceStateChanged(IceState),
    /// Local ICE candidate generated (needs to be sent to peer via signaling).
    LocalIceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
    },
    /// ICE gathering complete.
    IceGatheringComplete,
}

/// ICE connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceState {
    New,
    Checking,
    Connected,
    Completed,
    Failed,
    Disconnected,
    Closed,
}

/// Pending ICE candidate data (received before remote description is set).
#[derive(Clone, Debug)]
pub struct IceCandidateData {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_m_line_index: Option<u16>,
}

/// A WebRTC peer connection with two data channels.
pub struct WebRtcPeer {
    peer_id: PeerId,
    pc: RtcPeerConnection,
    state_channel: Option<RtcDataChannel>,
    events_channel: Option<RtcDataChannel>,
    incoming: Rc<EventQueue<PeerEvent>>,
}

impl WebRtcPeer {
    /// Create a new peer connection (without data channels yet).
    pub async fn new(peer_id: PeerId) -> Result<Self, JsValue> {
        let servers = fetch_turn_servers(peer_id).await;
        let ice_servers_array = to_js_ice_servers(&servers)?;

        let config = RtcConfiguration::new();
        config.set_ice_servers(&ice_servers_array);

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

        let pc = RtcPeerConnection::new_with_configuration(&config)?;
        let incoming = Rc::new(EventQueue::new());

        // Set up ICE candidate handler
        let incoming_clone = incoming.clone();
        let onicecandidate = Closure::wrap(Box::new(move |ev: JsValue| {
            let ev: RtcPeerConnectionIceEvent = ev.unchecked_into();
            if let Some(c) = ev.candidate() {
                log::info!(
                    "Generated ICE candidate for peer {}: {}",
                    peer_id,
                    c.candidate()
                );
                incoming_clone.push(PeerEvent::LocalIceCandidate {
                    candidate: c.candidate(),
                    sdp_mid: c.sdp_mid(),
                    sdp_m_line_index: c.sdp_m_line_index(),
                });
            } else {
                log::info!("ICE gathering complete for peer {}", peer_id);
                incoming_clone.push(PeerEvent::IceGatheringComplete);
            }
        }) as Box<dyn FnMut(JsValue)>);
        pc.set_onicecandidate(Some(onicecandidate.as_ref().unchecked_ref()));
        onicecandidate.forget();

        // Set up ICE connection state handler
        let incoming_clone = incoming.clone();
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

                let state = match ice_state {
                    web_sys::RtcIceConnectionState::New => {
                        net_log(
                            NetLogLevel::Info,
                            &format!("Peer {}: ICE starting", peer_id),
                        );
                        IceState::New
                    }
                    web_sys::RtcIceConnectionState::Checking => {
                        net_log(
                            NetLogLevel::Info,
                            &format!("Peer {}: Trying connection...", peer_id),
                        );
                        IceState::Checking
                    }
                    web_sys::RtcIceConnectionState::Connected => {
                        log::info!("Peer {} CONNECTED!", peer_id);
                        net_log(
                            NetLogLevel::Success,
                            &format!("Peer {}: Connected!", peer_id),
                        );
                        IceState::Connected
                    }
                    web_sys::RtcIceConnectionState::Completed => IceState::Completed,
                    web_sys::RtcIceConnectionState::Failed => {
                        log::error!("Peer {} connection FAILED!", peer_id);
                        net_log(
                            NetLogLevel::Error,
                            &format!("Peer {}: Connection failed", peer_id),
                        );
                        IceState::Failed
                    }
                    web_sys::RtcIceConnectionState::Disconnected => {
                        log::warn!("Peer {} disconnected", peer_id);
                        net_log(
                            NetLogLevel::Warning,
                            &format!("Peer {}: Disconnected", peer_id),
                        );
                        IceState::Disconnected
                    }
                    web_sys::RtcIceConnectionState::Closed => IceState::Closed,
                    _ => return,
                };

                incoming_clone.push(PeerEvent::IceStateChanged(state));
            }
        }) as Box<dyn FnMut(JsValue)>);
        pc.set_oniceconnectionstatechange(Some(onice.as_ref().unchecked_ref()));
        onice.forget();

        // Set up data channel handler (for incoming channels from remote peer)
        let incoming_clone = incoming.clone();
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

            let channel_kind = match label.as_str() {
                "state" => ChannelKind::State,
                "events" => ChannelKind::Events,
                _ => {
                    log::warn!("Unknown data channel: {}", label);
                    return;
                }
            };

            Self::setup_channel_callbacks(&dc, channel_kind, peer_id, &incoming_clone);

            // Store immediately in thread-local (like old design)
            RECEIVED_CHANNELS.with(|rc| {
                rc.borrow_mut().push(ReceivedChannel {
                    peer_id,
                    kind: channel_kind,
                    channel: dc,
                });
            });
            log::info!(
                "Queued received {:?} channel for peer {}",
                channel_kind,
                peer_id
            );
        }) as Box<dyn FnMut(JsValue)>);
        pc.set_ondatachannel(Some(ondc.as_ref().unchecked_ref()));
        ondc.forget();

        Ok(Self {
            peer_id,
            pc,
            state_channel: None,
            events_channel: None,
            incoming,
        })
    }

    /// Create data channels (call this when we're the initiator).
    pub fn create_data_channels(&mut self) {
        // Create unreliable channel for position updates
        let state_init = RtcDataChannelInit::new();
        state_init.set_ordered(false);
        state_init.set_max_retransmits(0);
        let state_dc = self
            .pc
            .create_data_channel_with_data_channel_dict("state", &state_init);
        Self::setup_channel_callbacks(&state_dc, ChannelKind::State, self.peer_id, &self.incoming);
        self.state_channel = Some(state_dc);

        // Create reliable channel for game events
        let events_dc = self.pc.create_data_channel("events");
        Self::setup_channel_callbacks(
            &events_dc,
            ChannelKind::Events,
            self.peer_id,
            &self.incoming,
        );
        self.events_channel = Some(events_dc);
    }

    /// Set up callbacks for a data channel.
    fn setup_channel_callbacks(
        dc: &RtcDataChannel,
        kind: ChannelKind,
        peer_id: PeerId,
        incoming: &Rc<EventQueue<PeerEvent>>,
    ) {
        let incoming_clone = incoming.clone();
        let onopen = Closure::wrap(Box::new(move |_: JsValue| {
            log::info!("{:?} channel open with peer {}", kind, peer_id);
            net_log(
                NetLogLevel::Success,
                &format!("Peer {}: {:?} channel ready", peer_id, kind),
            );
            incoming_clone.push(PeerEvent::ChannelOpened(kind));
        }) as Box<dyn FnMut(JsValue)>);
        dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        let incoming_clone = incoming.clone();
        let onmsg = Closure::wrap(Box::new(move |ev: JsValue| {
            let ev: MessageEvent = ev.unchecked_into();
            if let Some(data) = ev.data().as_string() {
                log::debug!("Received {:?} message from peer {}", kind, peer_id);
                incoming_clone.push(PeerEvent::Message {
                    channel: kind,
                    data,
                });
            } else {
                log::warn!(
                    "{:?} channel: received non-string data from peer {}",
                    kind,
                    peer_id
                );
            }
        }) as Box<dyn FnMut(JsValue)>);
        dc.set_onmessage(Some(onmsg.as_ref().unchecked_ref()));
        onmsg.forget();
    }

    /// Poll for incoming events from this peer.
    pub fn poll_events(&self) -> Vec<PeerEvent> {
        self.incoming.drain()
    }

    /// Store a received data channel (when we're the responder).
    pub fn store_channel(&mut self, kind: ChannelKind, dc: RtcDataChannel) {
        match kind {
            ChannelKind::State => self.state_channel = Some(dc),
            ChannelKind::Events => self.events_channel = Some(dc),
        }
    }

    /// Send data on a channel.
    pub fn send(&self, kind: ChannelKind, data: &str) -> Result<(), &'static str> {
        let channel = match kind {
            ChannelKind::State => &self.state_channel,
            ChannelKind::Events => &self.events_channel,
        };

        if let Some(dc) = channel {
            if dc.ready_state() == web_sys::RtcDataChannelState::Open {
                let _ = dc.send_with_str(data);
                Ok(())
            } else {
                Err("channel not open")
            }
        } else {
            Err("channel not created")
        }
    }

    /// Check if a channel is ready for sending.
    pub fn channel_ready(&self, kind: ChannelKind) -> bool {
        let channel = match kind {
            ChannelKind::State => &self.state_channel,
            ChannelKind::Events => &self.events_channel,
        };

        channel
            .as_ref()
            .is_some_and(|dc| dc.ready_state() == web_sys::RtcDataChannelState::Open)
    }

    /// Create an SDP offer.
    pub async fn create_offer(&self) -> Result<String, JsValue> {
        let offer = JsFuture::from(self.pc.create_offer()).await?;
        let sdp = get_sdp(&offer);
        let init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        init.set_sdp(&sdp);
        JsFuture::from(self.pc.set_local_description(&init)).await?;
        Ok(sdp)
    }

    /// Create an SDP answer (after receiving an offer).
    pub async fn create_answer(&self) -> Result<String, JsValue> {
        let answer = JsFuture::from(self.pc.create_answer()).await?;
        let sdp = get_sdp(&answer);
        let init = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        init.set_sdp(&sdp);
        JsFuture::from(self.pc.set_local_description(&init)).await?;
        Ok(sdp)
    }

    /// Set the remote SDP offer.
    pub async fn set_remote_offer(&self, sdp: &str) -> Result<(), JsValue> {
        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        desc.set_sdp(sdp);
        JsFuture::from(self.pc.set_remote_description(&desc)).await?;
        Ok(())
    }

    /// Get the underlying RtcPeerConnection (for stats).
    pub fn rtc_peer_connection(&self) -> &RtcPeerConnection {
        &self.pc
    }

    /// Close the connection.
    pub fn close(&self) {
        self.pc.close();
    }

    /// Check if we have a remote description set.
    pub fn has_remote_description(&self) -> bool {
        self.pc.remote_description().is_some()
    }
}

/// Extract SDP string from a JS session description object.
fn get_sdp(js: &JsValue) -> String {
    js_sys::Reflect::get(js, &"sdp".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}
