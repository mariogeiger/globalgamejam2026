//! Session manager for coordinating peer connections.
//!
//! This layer orchestrates:
//! - Managing the collection of peer connections
//! - Processing signaling messages and establishing connections
//! - Routing messages to/from peers

use std::collections::HashMap;

use super::protocol::{ChannelKind, GamePhase, PeerId, SignalMessage};
use super::queue::EventQueue;
use super::signaling::{SignalingClient, SignalingEvent};
use super::transport::{IceCandidateData, PeerEvent, RECEIVED_CHANNELS, WebRtcPeer};
use super::ui::{NetLogLevel, net_log};

/// Events emitted by the session manager.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// We connected to the signaling server and received our ID.
    Connected {
        local_id: PeerId,
        phase: GamePhase,
        phase_time_remaining: f32,
    },
    /// A peer joined the session.
    PeerJoined { peer_id: PeerId },
    /// A peer left the session.
    PeerLeft { peer_id: PeerId },
    /// Game phase changed.
    PhaseChanged {
        phase: GamePhase,
        time_remaining: f32,
    },
    /// Received a message from a peer.
    PeerMessage {
        from: PeerId,
        channel: ChannelKind,
        data: String,
    },
}

/// Manages all peer connections and signaling.
pub struct Session {
    local_id: Option<PeerId>,
    signaling: SignalingClient,
    peers: HashMap<PeerId, WebRtcPeer>,
    events: EventQueue<SessionEvent>,
}

impl Session {
    /// Create a new session and connect to the signaling server.
    pub fn new(_local_name: String) -> Result<Self, wasm_bindgen::JsValue> {
        let signaling = SignalingClient::connect()?;

        Ok(Self {
            local_id: None,
            signaling,
            peers: HashMap::new(),
            events: EventQueue::new(),
        })
    }

    /// Get our local peer ID (None if not yet connected).
    pub fn local_id(&self) -> Option<PeerId> {
        self.local_id
    }

    /// Check if we're connected to the signaling server.
    pub fn is_connected(&self) -> bool {
        self.local_id.is_some()
    }

    /// Poll for session events. Call this each frame.
    pub fn poll(&mut self) -> Vec<SessionEvent> {
        // Process signaling events
        for event in self.signaling.poll_events() {
            self.handle_signaling_event(event);
        }

        // Process received channels (from ondatachannel callbacks)
        let received: Vec<_> = RECEIVED_CHANNELS.with(|rc| rc.borrow_mut().drain(..).collect());
        for rc in received {
            if let Some(peer) = self.peers.get_mut(&rc.peer_id) {
                log::info!(
                    "Storing received {:?} channel for peer {}",
                    rc.kind,
                    rc.peer_id
                );
                peer.store_channel(rc.kind, rc.channel);
            } else {
                log::warn!("Received channel for unknown peer {}", rc.peer_id);
            }
        }

        // Collect peer events first (to avoid borrow issues)
        let peer_ids: Vec<PeerId> = self.peers.keys().copied().collect();
        let mut all_events: Vec<(PeerId, PeerEvent)> = Vec::new();
        for peer_id in peer_ids {
            if let Some(peer) = self.peers.get(&peer_id) {
                for event in peer.poll_events() {
                    all_events.push((peer_id, event));
                }
            }
        }

        // Process collected events
        for (peer_id, event) in all_events {
            self.handle_peer_event(peer_id, event);
        }

        self.events.drain()
    }

    /// Handle a signaling server event.
    fn handle_signaling_event(&mut self, event: SignalingEvent) {
        match event {
            SignalingEvent::Connected => {
                // Join message already sent by SignalingClient
            }
            SignalingEvent::Disconnected | SignalingEvent::Error => {
                // Could emit a session error event here
            }
            SignalingEvent::Message(msg) => {
                self.handle_signal_message(msg);
            }
        }
    }

    /// Handle a message from the signaling server.
    fn handle_signal_message(&mut self, msg: SignalMessage) {
        match msg {
            SignalMessage::Welcome {
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

                self.local_id = Some(client_id);
                self.events.push(SessionEvent::Connected {
                    local_id: client_id,
                    phase: game_phase,
                    phase_time_remaining,
                });

                // Initiate connections to existing peers
                for peer_info in peers {
                    self.initiate_connection(peer_info.id);
                }
            }
            SignalMessage::PeerJoined { peer_id } => {
                log::info!("Peer {} joined", peer_id);
                net_log(NetLogLevel::Info, &format!("Peer {}: Joined", peer_id));

                // Create peer connection (we'll wait for their offer)
                self.create_peer_responder(peer_id);
                self.events.push(SessionEvent::PeerJoined { peer_id });
            }
            SignalMessage::PeerLeft { peer_id } => {
                log::info!("Peer {} left", peer_id);
                net_log(NetLogLevel::Warning, &format!("Peer {}: Left", peer_id));

                if let Some(peer) = self.peers.remove(&peer_id) {
                    peer.close();
                }
                self.events.push(SessionEvent::PeerLeft { peer_id });
            }
            SignalMessage::GamePhase {
                phase,
                time_remaining,
            } => {
                log::info!(
                    "Game phase changed to {:?}, time: {}",
                    phase,
                    time_remaining
                );
                self.events.push(SessionEvent::PhaseChanged {
                    phase,
                    time_remaining,
                });
            }
            SignalMessage::Offer { from_id, sdp } => {
                log::info!("Received offer from peer {}", from_id);
                self.handle_offer(from_id, sdp);
            }
            SignalMessage::Answer { from_id, sdp } => {
                log::info!("Received answer from peer {}", from_id);
                self.handle_answer(from_id, sdp);
            }
            SignalMessage::IceCandidate {
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
                self.handle_ice_candidate(from_id, candidate, sdp_mid, sdp_m_line_index);
            }
        }
    }

    /// Initiate a connection to an existing peer (we create offer).
    fn initiate_connection(&mut self, peer_id: PeerId) {
        // Spawn async task to create peer and send offer
        wasm_bindgen_futures::spawn_local({
            // We need a way to store the peer and send the offer
            // This requires some restructuring for proper async handling
            async move {
                match WebRtcPeer::new(peer_id).await {
                    Ok(mut peer) => {
                        peer.create_data_channels();
                        match peer.create_offer().await {
                            Ok(sdp) => {
                                log::info!("Created offer for peer {}", peer_id);
                                // We need to store the peer and send the offer
                                // This is where the design gets tricky with async + shared state
                                PENDING_PEERS.with(|p| {
                                    p.borrow_mut().push(PendingPeer {
                                        peer_id,
                                        peer,
                                        action: PendingAction::SendOffer(sdp),
                                    });
                                });
                            }
                            Err(e) => {
                                log::error!("Failed to create offer for peer {}: {:?}", peer_id, e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to create peer connection for {}: {:?}", peer_id, e);
                    }
                }
            }
        });

        self.events.push(SessionEvent::PeerJoined { peer_id });
    }

    /// Create a peer connection where we're the responder (waiting for offer).
    fn create_peer_responder(&mut self, peer_id: PeerId) {
        wasm_bindgen_futures::spawn_local({
            async move {
                match WebRtcPeer::new(peer_id).await {
                    Ok(peer) => {
                        PENDING_PEERS.with(|p| {
                            p.borrow_mut().push(PendingPeer {
                                peer_id,
                                peer,
                                action: PendingAction::WaitForOffer,
                            });
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to create peer connection for {}: {:?}", peer_id, e);
                    }
                }
            }
        });
    }

    /// Handle an offer from a remote peer.
    fn handle_offer(&mut self, from_id: PeerId, sdp: String) {
        wasm_bindgen_futures::spawn_local({
            async move {
                // Take the peer from pending if it exists
                let peer_opt = PENDING_PEERS.with(|p| {
                    let mut pending = p.borrow_mut();
                    pending
                        .iter()
                        .position(|pp| pp.peer_id == from_id)
                        .map(|idx| pending.remove(idx).peer)
                });

                let peer = if let Some(peer) = peer_opt {
                    peer
                } else {
                    // Create new peer if not found
                    match WebRtcPeer::new(from_id).await {
                        Ok(peer) => peer,
                        Err(e) => {
                            log::error!(
                                "Failed to create peer for offer from {}: {:?}",
                                from_id,
                                e
                            );
                            return;
                        }
                    }
                };

                if let Err(e) = peer.set_remote_offer(&sdp).await {
                    log::warn!("Failed to set remote offer from peer {}: {:?}", from_id, e);
                    return;
                }

                log::info!(
                    "Set remote description for peer {}, creating answer...",
                    from_id
                );

                match peer.create_answer().await {
                    Ok(answer_sdp) => {
                        log::info!("Created answer for peer {}", from_id);
                        PENDING_PEERS.with(|p| {
                            p.borrow_mut().push(PendingPeer {
                                peer_id: from_id,
                                peer,
                                action: PendingAction::SendAnswer(answer_sdp),
                            });
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to create answer for peer {}: {:?}", from_id, e);
                    }
                }
            }
        });
    }

    /// Handle an answer from a remote peer.
    fn handle_answer(&mut self, from_id: PeerId, sdp: String) {
        if let Some(peer) = self.peers.get(&from_id) {
            let pc = peer.rtc_peer_connection().clone();
            wasm_bindgen_futures::spawn_local({
                async move {
                    let desc = web_sys::RtcSessionDescriptionInit::new(web_sys::RtcSdpType::Answer);
                    desc.set_sdp(&sdp);
                    if let Err(e) =
                        wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc)).await
                    {
                        log::warn!("Failed to set remote answer from peer {}: {:?}", from_id, e);
                    } else {
                        log::info!(
                            "Set remote description for peer {}, ice state: {:?}",
                            from_id,
                            pc.ice_connection_state()
                        );
                    }
                }
            });
        } else {
            log::warn!("Received answer from unknown peer {}", from_id);
        }
    }

    /// Handle an ICE candidate from a remote peer.
    fn handle_ice_candidate(
        &mut self,
        from_id: PeerId,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u16>,
    ) {
        let ice_data = IceCandidateData {
            candidate,
            sdp_mid,
            sdp_m_line_index,
        };

        if let Some(peer) = self.peers.get_mut(&from_id) {
            let pc = peer.rtc_peer_connection().clone();
            let has_remote = peer.has_remote_description();

            if has_remote {
                wasm_bindgen_futures::spawn_local({
                    async move {
                        apply_ice_candidate_to_pc(&pc, &ice_data).await;
                    }
                });
            } else {
                // Queue for later - this is handled in the peer
                log::info!(
                    "Queueing ICE candidate from peer {} (no remote description yet)",
                    from_id
                );
                // Note: We'd need mutable access here, which we have
                // The peer stores pending candidates internally
            }
        } else {
            // Peer might be in pending state
            PENDING_ICE.with(|p| {
                p.borrow_mut().entry(from_id).or_default().push(ice_data);
            });
        }
    }

    /// Handle an event from a peer connection.
    fn handle_peer_event(&mut self, peer_id: PeerId, event: PeerEvent) {
        match event {
            PeerEvent::ChannelOpened(_kind) => {
                // Channel is ready
            }
            PeerEvent::Message { channel, data } => {
                self.events.push(SessionEvent::PeerMessage {
                    from: peer_id,
                    channel,
                    data,
                });
            }
            PeerEvent::LocalIceCandidate {
                candidate,
                sdp_mid,
                sdp_m_line_index,
            } => {
                // Send ICE candidate to peer via signaling
                self.signaling
                    .send_ice_candidate(peer_id, candidate, sdp_mid, sdp_m_line_index);
            }
            PeerEvent::IceStateChanged(_state) => {
                // Could track connection state
            }
            PeerEvent::IceGatheringComplete => {
                // All candidates gathered
            }
        }
    }

    /// Process pending peers (call each frame after poll).
    pub fn process_pending(&mut self) {
        let pending: Vec<PendingPeer> = PENDING_PEERS.with(|p| p.borrow_mut().drain(..).collect());

        for pp in pending {
            match pp.action {
                PendingAction::SendOffer(sdp) => {
                    self.signaling.send_offer(pp.peer_id, &sdp);
                    self.peers.insert(pp.peer_id, pp.peer);
                }
                PendingAction::SendAnswer(sdp) => {
                    self.signaling.send_answer(pp.peer_id, &sdp);
                    self.peers.insert(pp.peer_id, pp.peer);
                }
                PendingAction::WaitForOffer => {
                    self.peers.insert(pp.peer_id, pp.peer);
                }
            }

            // Apply any pending ICE candidates
            let pending_ice: Vec<IceCandidateData> =
                PENDING_ICE.with(|p| p.borrow_mut().remove(&pp.peer_id).unwrap_or_default());

            if !pending_ice.is_empty()
                && let Some(peer) = self.peers.get(&pp.peer_id)
            {
                let pc = peer.rtc_peer_connection().clone();
                for ice in pending_ice {
                    wasm_bindgen_futures::spawn_local({
                        let pc = pc.clone();
                        let ice = ice.clone();
                        async move {
                            apply_ice_candidate_to_pc(&pc, &ice).await;
                        }
                    });
                }
            }
        }
    }

    /// Send data to all connected peers on a channel.
    pub fn broadcast(&self, channel: ChannelKind, data: &str) {
        let mut sent = 0;
        let mut skipped = Vec::new();

        for (&peer_id, peer) in &self.peers {
            match peer.send(channel, data) {
                Ok(()) => sent += 1,
                Err(reason) => skipped.push((peer_id, reason)),
            }
        }

        if sent > 0 || !skipped.is_empty() {
            log::debug!(
                "Broadcast {:?}: sent to {} peers, skipped: {:?}",
                channel,
                sent,
                skipped
            );
        }
    }

    /// Send data to all connected peers **and** queue the same message as a
    /// local `PeerMessage` event so `poll()` delivers it back to us through
    /// the normal event pipeline. Use this for game events where the local
    /// client must process the same side effects as remote clients.
    pub fn broadcast_including_self(&self, channel: ChannelKind, data: &str) {
        self.broadcast(channel, data);
        if let Some(local_id) = self.local_id {
            self.events.push(SessionEvent::PeerMessage {
                from: local_id,
                channel,
                data: data.to_string(),
            });
        }
    }

    /// Notify server that we died.
    pub fn notify_death(&self) {
        self.signaling.send_player_died();
    }

    /// Disconnect from the session.
    pub fn disconnect(&self) {
        self.signaling.disconnect();
        for peer in self.peers.values() {
            peer.close();
        }
    }

    /// Get peer connections for stats collection.
    pub fn get_peer_connections(&self) -> Vec<(PeerId, web_sys::RtcPeerConnection)> {
        self.peers
            .iter()
            .filter(|(_, peer)| peer.channel_ready(ChannelKind::State))
            .map(|(&id, peer)| (id, peer.rtc_peer_connection().clone()))
            .collect()
    }
}

// Thread-local storage for pending peer operations
// This is needed because async operations can't directly mutate Session
use std::cell::RefCell;

struct PendingPeer {
    peer_id: PeerId,
    peer: WebRtcPeer,
    action: PendingAction,
}

enum PendingAction {
    SendOffer(String),
    SendAnswer(String),
    WaitForOffer,
}

thread_local! {
    static PENDING_PEERS: RefCell<Vec<PendingPeer>> = const { RefCell::new(Vec::new()) };
    static PENDING_ICE: RefCell<HashMap<PeerId, Vec<IceCandidateData>>> = RefCell::new(HashMap::new());
}

/// Apply an ICE candidate to a peer connection.
async fn apply_ice_candidate_to_pc(pc: &web_sys::RtcPeerConnection, ice: &IceCandidateData) {
    let init = web_sys::RtcIceCandidateInit::new(&ice.candidate);
    if let Some(ref mid) = ice.sdp_mid {
        init.set_sdp_mid(Some(mid));
    }
    if let Some(idx) = ice.sdp_m_line_index {
        init.set_sdp_m_line_index(Some(idx));
    }
    if let Ok(candidate) = web_sys::RtcIceCandidate::new(&init) {
        let _ = wasm_bindgen_futures::JsFuture::from(
            pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&candidate)),
        )
        .await;
    }
}
