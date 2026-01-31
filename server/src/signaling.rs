use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    A,
    B,
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
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "welcome")]
    Welcome {
        #[serde(rename = "clientId")]
        client_id: ClientId,
        team: Team,
        peers: Vec<PeerInfo>,
    },
    #[serde(rename = "peer-joined")]
    PeerJoined {
        #[serde(rename = "peerId")]
        peer_id: ClientId,
        team: Team,
    },
    #[serde(rename = "peer-left")]
    PeerLeft {
        #[serde(rename = "peerId")]
        peer_id: ClientId,
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
    team: Team,
}

type ClientId = u64;
type ClientSender = mpsc::UnboundedSender<String>;

struct ClientInfo {
    team: Team,
    sender: ClientSender,
}

struct SignalingState {
    next_id: ClientId,
    clients: HashMap<ClientId, ClientInfo>,
}

impl SignalingState {
    fn new() -> Self {
        Self {
            next_id: 0,
            clients: HashMap::new(),
        }
    }

    fn assign_team(&self) -> Team {
        let team_a_count = self.clients.values().filter(|c| c.team == Team::A).count();
        let team_b_count = self.clients.values().filter(|c| c.team == Team::B).count();
        if team_a_count <= team_b_count {
            Team::A
        } else {
            Team::B
        }
    }
}

type SharedState = Arc<Mutex<SignalingState>>;

static STATE: std::sync::OnceLock<SharedState> = std::sync::OnceLock::new();

fn get_state() -> &'static SharedState {
    STATE.get_or_init(|| Arc::new(Mutex::new(SignalingState::new())))
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

            // Assign team (balance teams)
            let team = s.assign_team();

            // Get list of existing peers
            let peers: Vec<PeerInfo> = s
                .clients
                .iter()
                .map(|(&id, info)| PeerInfo {
                    id,
                    team: info.team,
                })
                .collect();

            log::info!(
                "Client {} joined as Team {:?}, {} existing peers",
                client_id,
                team,
                peers.len()
            );

            // Broadcast peer-joined to all existing clients
            let peer_joined = ServerMessage::PeerJoined {
                peer_id: client_id,
                team,
            };
            if let Ok(json) = serde_json::to_string(&peer_joined) {
                for info in s.clients.values() {
                    let _ = info.sender.send(json.clone());
                }
            }

            // Add this client to the room
            s.clients.insert(
                client_id,
                ClientInfo {
                    team,
                    sender: sender.clone(),
                },
            );

            // Send welcome message to the new client
            let welcome = ServerMessage::Welcome {
                client_id,
                team,
                peers,
            };
            if let Ok(json) = serde_json::to_string(&welcome) {
                let _ = sender.send(json);
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
