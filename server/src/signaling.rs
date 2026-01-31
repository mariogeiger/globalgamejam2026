use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct SignalMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
    #[serde(rename = "sdpMid", skip_serializing_if = "Option::is_none")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex", skip_serializing_if = "Option::is_none")]
    sdp_m_line_index: Option<u16>,
}

type ClientId = u64;
type ClientSender = mpsc::UnboundedSender<String>;

struct SignalingState {
    next_id: ClientId,
    waiting_client: Option<ClientId>,
    peers: HashMap<ClientId, ClientId>,
    senders: HashMap<ClientId, ClientSender>,
}

impl SignalingState {
    fn new() -> Self {
        Self {
            next_id: 0,
            waiting_client: None,
            peers: HashMap::new(),
            senders: HashMap::new(),
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
    state.lock().await.senders.insert(client_id, tx);

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

    // Receive messages from client
    while let Some(Ok(msg)) = ws_rx.next().await {
        if let Message::Text(text) = msg
            && let Ok(signal) = serde_json::from_str::<SignalMessage>(&text)
        {
            handle_message(client_id, signal, state).await;
        }
    }

    // Cleanup
    log::info!("Client {} disconnected", client_id);
    cleanup_client(client_id, state).await;
    send_task.abort();
}

async fn handle_message(client_id: ClientId, msg: SignalMessage, state: &SharedState) {
    log::info!("Client {} sent: {}", client_id, msg.msg_type);

    match msg.msg_type.as_str() {
        "join" => {
            let mut s = state.lock().await;

            if let Some(waiting_id) = s.waiting_client.take() {
                s.peers.insert(client_id, waiting_id);
                s.peers.insert(waiting_id, client_id);

                log::info!("Paired clients {} and {}", client_id, waiting_id);

                send_to_client_locked(&s, waiting_id, r#"{"type":"create-offer"}"#);
                send_to_client_locked(&s, client_id, r#"{"type":"waiting-for-offer"}"#);
            } else {
                s.waiting_client = Some(client_id);
                log::info!("Client {} waiting for peer", client_id);
                send_to_client_locked(&s, client_id, r#"{"type":"waiting"}"#);
            }
        }
        "offer" | "answer" | "ice-candidate" => {
            let s = state.lock().await;
            if let Some(&peer_id) = s.peers.get(&client_id)
                && let Ok(json) = serde_json::to_string(&msg)
            {
                send_to_client_locked(&s, peer_id, &json);
            }
        }
        _ => {}
    }
}

fn send_to_client_locked(state: &SignalingState, client_id: ClientId, msg: &str) {
    if let Some(tx) = state.senders.get(&client_id) {
        let _ = tx.send(msg.to_string());
    }
}

async fn cleanup_client(client_id: ClientId, state: &SharedState) {
    let mut s = state.lock().await;

    s.senders.remove(&client_id);

    if s.waiting_client == Some(client_id) {
        s.waiting_client = None;
    }

    if let Some(peer_id) = s.peers.remove(&client_id) {
        s.peers.remove(&peer_id);
        send_to_client_locked(&s, peer_id, r#"{"type":"peer-disconnected"}"#);
    }
}
