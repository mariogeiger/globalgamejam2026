//! Network protocol message types and serialization.
//!
//! This module contains all message types used for:
//! - Peer-to-peer communication (state updates, game events)
//! - Signaling server communication (offers, answers, ICE candidates)

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Unique identifier for a connected peer.
pub type PeerId = u64;

/// Game phase, synchronized by the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    WaitingForPlayers,
    GracePeriod,
    Playing,
    Victory,
    Spectating,
}

/// Which data channel to use for sending messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Unreliable, unordered channel for high-frequency position updates.
    State,
    /// Reliable, ordered channel for game events (kills, introductions).
    Events,
}

// ============================================================================
// Peer-to-peer messages
// ============================================================================

/// Player state update sent on the unreliable "state" channel.
/// Sent at high frequency (~20Hz) for position synchronization.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StateUpdate {
    #[serde(rename = "msg_type")]
    msg_type: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
    #[serde(default)]
    pub mask: u8,
}

impl StateUpdate {
    pub fn new(position: Vec3, yaw: f32, pitch: f32, mask: u8) -> Self {
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

    pub fn position(&self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
}

/// Game event messages sent on the reliable "events" channel.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum GameMessage {
    /// Notification that we killed another player.
    #[serde(rename = "kill")]
    Kill { victim_id: PeerId },

    /// Introduction with our player name.
    #[serde(rename = "introduction")]
    Introduction { name: String },
}

// ============================================================================
// Signaling server messages (incoming)
// ============================================================================

/// Basic peer info from the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PeerInfo {
    pub id: PeerId,
}

/// Messages received from the signaling server.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SignalMessage {
    /// Welcome message with our ID and list of existing peers.
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

    /// A new peer joined the game.
    #[serde(rename = "peer-joined")]
    PeerJoined {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
    },

    /// A peer left the game.
    #[serde(rename = "peer-left")]
    PeerLeft {
        #[serde(rename = "peerId")]
        peer_id: PeerId,
    },

    /// Game phase changed (from server).
    #[serde(rename = "game-phase")]
    GamePhase {
        phase: GamePhase,
        #[serde(rename = "timeRemaining")]
        time_remaining: f32,
    },

    /// WebRTC offer from another peer.
    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "fromId")]
        from_id: PeerId,
        sdp: String,
    },

    /// WebRTC answer from another peer.
    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "fromId")]
        from_id: PeerId,
        sdp: String,
    },

    /// ICE candidate from another peer.
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

// ============================================================================
// Signaling server commands (outgoing)
// ============================================================================

/// Commands sent to the signaling server.
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SignalCommand {
    /// Join the game.
    #[serde(rename = "join")]
    Join,

    /// Leave the game.
    #[serde(rename = "leave")]
    Leave,

    /// Notify server that we died.
    #[serde(rename = "player_died")]
    PlayerDied,

    /// Send WebRTC offer to a peer.
    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "targetId")]
        target_id: PeerId,
        sdp: String,
    },

    /// Send WebRTC answer to a peer.
    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "targetId")]
        target_id: PeerId,
        sdp: String,
    },

    /// Send ICE candidate to a peer.
    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "targetId")]
        target_id: PeerId,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
    },
}
