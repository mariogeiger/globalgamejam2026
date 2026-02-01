use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};

const GRACE_PERIOD_DURATION: f32 = 10.0;
const VICTORY_DURATION: f32 = 10.0;
const MIN_PLAYERS_TO_START: usize = 2;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    WaitingForPlayers,
    GracePeriod,
    Playing,
    Victory,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "join")]
    Join,
    #[serde(rename = "offer")]
    Offer {
        #[serde(rename = "targetId")]
        target_id: ClientId,
        sdp: String,
    },
    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "targetId")]
        target_id: ClientId,
        sdp: String,
    },
    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "targetId")]
        target_id: ClientId,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
    },
    #[serde(rename = "player_died")]
    PlayerDied,
    #[serde(rename = "leave")]
    Leave,
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "welcome")]
    Welcome {
        #[serde(rename = "clientId")]
        client_id: ClientId,
        peers: Vec<PeerInfo>,
        #[serde(rename = "gamePhase")]
        game_phase: GamePhase,
        #[serde(rename = "phaseTimeRemaining")]
        phase_time_remaining: f32,
    },
    #[serde(rename = "peer-joined")]
    PeerJoined {
        #[serde(rename = "peerId")]
        peer_id: ClientId,
    },
    #[serde(rename = "peer-left")]
    PeerLeft {
        #[serde(rename = "peerId")]
        peer_id: ClientId,
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
        from_id: ClientId,
        sdp: String,
    },
    #[serde(rename = "answer")]
    Answer {
        #[serde(rename = "fromId")]
        from_id: ClientId,
        sdp: String,
    },
    #[serde(rename = "ice-candidate")]
    IceCandidate {
        #[serde(rename = "fromId")]
        from_id: ClientId,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
    },
}

#[derive(Serialize, Debug, Clone)]
struct PeerInfo {
    id: ClientId,
}

type ClientId = u64;
type ClientSender = mpsc::UnboundedSender<String>;

struct ClientInfo {
    sender: ClientSender,
    is_alive: bool,
}

struct SignalingState {
    next_id: ClientId,
    clients: HashMap<ClientId, ClientInfo>,
    game_phase: GamePhase,
    phase_start: Instant,
    phase_duration: f32,
}

impl SignalingState {
    fn new() -> Self {
        Self {
            next_id: 0,
            clients: HashMap::new(),
            game_phase: GamePhase::WaitingForPlayers,
            phase_start: Instant::now(),
            phase_duration: 0.0,
        }
    }

    fn phase_time_remaining(&self) -> f32 {
        let elapsed = self.phase_start.elapsed().as_secs_f32();
        (self.phase_duration - elapsed).max(0.0)
    }

    fn broadcast(&self, msg: &ServerMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            for info in self.clients.values() {
                let _ = info.sender.send(json.clone());
            }
        }
    }

    fn set_phase(&mut self, phase: GamePhase, duration: f32) {
        self.game_phase = phase;
        self.phase_start = Instant::now();
        self.phase_duration = duration;

        // Reset all players to alive when starting a new round
        if phase == GamePhase::GracePeriod {
            for client in self.clients.values_mut() {
                client.is_alive = true;
            }
        }

        log::info!("Game phase changed to {:?}, duration: {}s", phase, duration);

        self.broadcast(&ServerMessage::GamePhase {
            phase,
            time_remaining: duration,
        });
    }

    fn alive_count(&self) -> usize {
        self.clients.values().filter(|c| c.is_alive).count()
    }
}

type SharedState = Arc<Mutex<SignalingState>>;

static STATE: std::sync::OnceLock<SharedState> = std::sync::OnceLock::new();

fn get_state() -> &'static SharedState {
    STATE.get_or_init(|| Arc::new(Mutex::new(SignalingState::new())))
}

pub fn start_game_loop() {
    let state = get_state().clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            update_game_state(&state).await;
        }
    });
}

async fn update_game_state(state: &SharedState) {
    let mut s = state.lock().await;

    let time_remaining = s.phase_time_remaining();
    let player_count = s.clients.len();
    let alive_count = s.alive_count();

    match s.game_phase {
        GamePhase::WaitingForPlayers => {
            if player_count >= MIN_PLAYERS_TO_START {
                s.set_phase(GamePhase::GracePeriod, GRACE_PERIOD_DURATION);
            }
        }
        GamePhase::GracePeriod => {
            if time_remaining <= 0.0 {
                s.set_phase(GamePhase::Playing, 0.0); // No time limit for playing
            }
        }
        GamePhase::Playing => {
            // Check for victory condition
            if player_count >= MIN_PLAYERS_TO_START && alive_count <= 1 {
                s.set_phase(GamePhase::Victory, VICTORY_DURATION);
            } else if player_count < MIN_PLAYERS_TO_START {
                // Not enough players, go back to waiting
                s.set_phase(GamePhase::WaitingForPlayers, 0.0);
            }
        }
        GamePhase::Victory => {
            if time_remaining <= 0.0 {
                if player_count >= MIN_PLAYERS_TO_START {
                    s.set_phase(GamePhase::GracePeriod, GRACE_PERIOD_DURATION);
                } else {
                    s.set_phase(GamePhase::WaitingForPlayers, 0.0);
                }
            }
        }
    }
}

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let state = get_state();

    let client_id = {
        let mut s = state.lock().await;
        let id = s.next_id;
        s.next_id += 1;
        id
    };

    log::info!("Client {} connected", client_id);

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let (mut ws_tx, mut ws_rx) = socket.split();

    use futures_util::{SinkExt, StreamExt};

    // Task to send messages to client
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Store sender temporarily (will be moved to clients map on join)
    let sender = tx;

    // Receive messages from client
    while let Some(Ok(msg)) = ws_rx.next().await {
        if let Message::Text(text) = msg
            && let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text)
        {
            handle_message(client_id, client_msg, state, sender.clone()).await;
        }
    }

    // Cleanup
    log::info!("Client {} disconnected", client_id);
    cleanup_client(client_id, state).await;
    send_task.abort();
}

async fn handle_message(
    client_id: ClientId,
    msg: ClientMessage,
    state: &SharedState,
    sender: ClientSender,
) {
    match msg {
        ClientMessage::Join => {
            let mut s = state.lock().await;

            // Get list of existing peers
            let peers: Vec<PeerInfo> = s.clients.iter().map(|(&id, _)| PeerInfo { id }).collect();

            log::info!(
                "Client {} joined, {} existing peers, phase: {:?}",
                client_id,
                peers.len(),
                s.game_phase
            );

            // Broadcast peer-joined to all existing clients
            let peer_joined = ServerMessage::PeerJoined { peer_id: client_id };
            if let Ok(json) = serde_json::to_string(&peer_joined) {
                for info in s.clients.values() {
                    let _ = info.sender.send(json.clone());
                }
            }

            // Add this client to the room
            // Late joiners start as dead if game is in progress
            let is_alive = matches!(
                s.game_phase,
                GamePhase::WaitingForPlayers | GamePhase::GracePeriod
            );

            s.clients.insert(
                client_id,
                ClientInfo {
                    sender: sender.clone(),
                    is_alive,
                },
            );

            // Send welcome message to the new client with game state
            let welcome = ServerMessage::Welcome {
                client_id,
                peers,
                game_phase: s.game_phase,
                phase_time_remaining: s.phase_time_remaining(),
            };
            if let Ok(json) = serde_json::to_string(&welcome) {
                let _ = sender.send(json);
            }
        }
        ClientMessage::PlayerDied => {
            let mut s = state.lock().await;
            if let Some(client) = s.clients.get_mut(&client_id) {
                client.is_alive = false;
                log::info!("Client {} died, {} alive", client_id, s.alive_count());
            }
        }
        ClientMessage::Offer { target_id, sdp } => {
            let s = state.lock().await;
            if let Some(target) = s.clients.get(&target_id) {
                let msg = ServerMessage::Offer {
                    from_id: client_id,
                    sdp,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = target.sender.send(json);
                }
            }
        }
        ClientMessage::Answer { target_id, sdp } => {
            let s = state.lock().await;
            if let Some(target) = s.clients.get(&target_id) {
                let msg = ServerMessage::Answer {
                    from_id: client_id,
                    sdp,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = target.sender.send(json);
                }
            }
        }
        ClientMessage::IceCandidate {
            target_id,
            candidate,
            sdp_mid,
            sdp_m_line_index,
        } => {
            let s = state.lock().await;
            if let Some(target) = s.clients.get(&target_id) {
                let msg = ServerMessage::IceCandidate {
                    from_id: client_id,
                    candidate,
                    sdp_mid,
                    sdp_m_line_index,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = target.sender.send(json);
                }
            }
        }
        ClientMessage::Leave => {
            log::info!("Client {} requested disconnect (AFK)", client_id);
        }
    }
}

async fn cleanup_client(client_id: ClientId, state: &SharedState) {
    let mut s = state.lock().await;

    s.clients.remove(&client_id);

    // Notify all remaining clients that this peer left
    let peer_left = ServerMessage::PeerLeft { peer_id: client_id };
    if let Ok(json) = serde_json::to_string(&peer_left) {
        for info in s.clients.values() {
            let _ = info.sender.send(json.clone());
        }
    }

    log::info!(
        "Client {} removed, {} clients remaining",
        client_id,
        s.clients.len()
    );
}
