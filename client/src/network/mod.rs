//! Network module for multiplayer game communication.
//!
//! This module provides a clean API for game code to interact with the network layer.
//! It handles WebRTC peer-to-peer connections via a signaling server.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  Game Code (game.rs)                    │
//! │  - Calls network.broadcast_state()      │
//! │  - Polls network.poll()                 │
//! └───────────────────┬─────────────────────┘
//!                     │
//! ┌───────────────────▼─────────────────────┐
//! │  GameNetwork (this module)              │
//! │  - Public API for game                  │
//! │  - Translates game actions ↔ protocol   │
//! └───────────────────┬─────────────────────┘
//!                     │
//! ┌───────────────────▼─────────────────────┐
//! │  Session (session.rs)                   │
//! │  - Manages peer collection              │
//! │  - Handles signaling protocol           │
//! └───────────────────┬─────────────────────┘
//!                     │
//! ┌───────────────────▼─────────────────────┐
//! │  Transport (transport.rs)               │
//! │  - WebRTC peer connections              │
//! │  - Data channels                        │
//! └─────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // Create network client
//! let network = GameNetwork::new("Player Name".to_string())?;
//!
//! // Each frame:
//! for event in network.poll() {
//!     match event {
//!         NetworkEvent::PlayerState { id, position, .. } => { /* update remote player */ }
//!         NetworkEvent::PlayerKilled { killer_id, victim_id } => { /* handle kill */ }
//!         // ...
//!     }
//! }
//!
//! // Send state
//! network.broadcast_state(position, yaw, pitch, mask);
//! network.send_kill(victim_id);
//! ```

mod ice;
mod protocol;
mod queue;
mod session;
mod signaling;
mod stats;
mod transport;
mod ui;

use glam::Vec3;
use web_sys::RtcPeerConnection;

use protocol::{ChannelKind, GameMessage, StateUpdate};
use session::{Session, SessionEvent};

// Re-export public types
pub use protocol::{GamePhase, PeerId};
pub use stats::{fetch_peer_stats, update_peer_stats_display};

/// Events emitted by the network layer for game code to handle.
#[derive(Clone, Debug)]
pub enum NetworkEvent {
    /// Connected to the signaling server with our ID.
    Connected {
        id: PeerId,
        phase: GamePhase,
        phase_time_remaining: f32,
    },
    /// A peer joined the game.
    PeerJoined { id: PeerId },
    /// A peer left the game.
    PeerLeft { id: PeerId },
    /// Game phase changed (from server).
    GamePhaseChanged {
        phase: GamePhase,
        time_remaining: f32,
    },
    /// Received player state update from a peer.
    PlayerState {
        id: PeerId,
        position: Vec3,
        yaw: f32,
        pitch: f32,
        mask: u8,
    },
    /// A player was killed.
    PlayerKilled {
        killer_id: PeerId,
        victim_id: PeerId,
    },
    /// A peer introduced themselves with their name.
    PeerIntroduction { id: PeerId, name: String },
}

/// Main network client for game code.
///
/// Provides a clean API for:
/// - Broadcasting player state to all peers
/// - Sending game events (kills)
/// - Receiving events from peers and the server
pub struct NetworkClient {
    session: Session,
}

impl NetworkClient {
    /// Create a new network client and connect to the signaling server.
    pub fn new(player_name: String) -> Result<Self, wasm_bindgen::JsValue> {
        let session = Session::new(player_name)?;
        Ok(Self { session })
    }

    /// Poll for network events. Call this each frame.
    pub fn poll_events(&mut self) -> Vec<NetworkEvent> {
        // Process pending async operations
        self.session.process_pending();

        // Poll session for events
        let session_events = self.session.poll();

        // Translate session events to network events
        session_events
            .into_iter()
            .filter_map(|event| self.translate_event(event))
            .collect()
    }

    /// Translate a session event to a network event.
    fn translate_event(&self, event: SessionEvent) -> Option<NetworkEvent> {
        match event {
            SessionEvent::Connected {
                local_id,
                phase,
                phase_time_remaining,
            } => Some(NetworkEvent::Connected {
                id: local_id,
                phase,
                phase_time_remaining,
            }),
            SessionEvent::PeerJoined { peer_id } => Some(NetworkEvent::PeerJoined { id: peer_id }),
            SessionEvent::PeerLeft { peer_id } => Some(NetworkEvent::PeerLeft { id: peer_id }),
            SessionEvent::PhaseChanged {
                phase,
                time_remaining,
            } => Some(NetworkEvent::GamePhaseChanged {
                phase,
                time_remaining,
            }),
            SessionEvent::PeerMessage {
                from,
                channel,
                data,
            } => self.parse_peer_message(from, channel, &data),
        }
    }

    /// Parse a message received from a peer.
    fn parse_peer_message(
        &self,
        from: PeerId,
        channel: ChannelKind,
        data: &str,
    ) -> Option<NetworkEvent> {
        match channel {
            ChannelKind::State => {
                // Parse state update
                match serde_json::from_str::<StateUpdate>(data) {
                    Ok(state) => Some(NetworkEvent::PlayerState {
                        id: from,
                        position: state.position(),
                        yaw: state.yaw,
                        pitch: state.pitch,
                        mask: state.mask,
                    }),
                    Err(e) => {
                        log::warn!("Failed to parse state from peer {}: {}", from, e);
                        None
                    }
                }
            }
            ChannelKind::Events => {
                // Parse game event
                match serde_json::from_str::<GameMessage>(data) {
                    Ok(msg) => match msg {
                        GameMessage::Kill { victim_id } => Some(NetworkEvent::PlayerKilled {
                            killer_id: from,
                            victim_id,
                        }),
                        GameMessage::Introduction { name } => {
                            Some(NetworkEvent::PeerIntroduction { id: from, name })
                        }
                    },
                    Err(e) => {
                        log::warn!("Failed to parse event from peer {}: {}", from, e);
                        None
                    }
                }
            }
        }
    }

    /// Broadcast player state to all connected peers.
    ///
    /// This is sent on the unreliable channel for low latency.
    pub fn send_player_state(&self, position: Vec3, yaw: f32, pitch: f32, mask: u8) {
        let state = StateUpdate::new(position, yaw, pitch, mask);
        if let Ok(json) = serde_json::to_string(&state) {
            self.session.broadcast(ChannelKind::State, &json);
        }
    }

    /// Send a kill notification to all peers.
    ///
    /// This is sent on the reliable channel to ensure delivery.
    pub fn send_kill(&self, victim_id: PeerId) {
        let msg = GameMessage::Kill { victim_id };
        if let Ok(json) = serde_json::to_string(&msg) {
            self.session.broadcast(ChannelKind::Events, &json);
        }
    }

    /// Get our local peer ID (None if not yet connected).
    pub fn local_id(&self) -> Option<PeerId> {
        self.session.local_id()
    }

    /// Check if we're connected to the signaling server.
    pub fn is_connected(&self) -> bool {
        self.session.is_connected()
    }

    /// Notify the server that we died.
    pub fn notify_death(&self) {
        self.session.notify_death();
    }

    /// Disconnect from the network.
    pub fn disconnect(&self) {
        self.session.disconnect();
    }

    /// Get peer connections for stats collection.
    pub fn get_peer_connections(&self) -> Vec<(PeerId, RtcPeerConnection)> {
        self.session.get_peer_connections()
    }
}
