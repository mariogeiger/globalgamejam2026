/// WebSocket signaling server for WebRTC peer pairing
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use tungstenite::{Message, accept};

#[derive(Serialize, Deserialize, Debug)]
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

struct SignalingState {
    next_id: ClientId,
    waiting_client: Option<ClientId>,
    peers: HashMap<ClientId, ClientId>,
}

impl SignalingState {
    fn new() -> Self {
        Self {
            next_id: 0,
            waiting_client: None,
            peers: HashMap::new(),
        }
    }
}

type SharedState = Arc<Mutex<SignalingState>>;
type ClientSenders = Arc<Mutex<HashMap<ClientId, std::sync::mpsc::Sender<String>>>>;

pub fn run_signaling_server(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;
    log::info!("WebSocket signaling server running on port {}", port);

    let state: SharedState = Arc::new(Mutex::new(SignalingState::new()));
    let senders: ClientSenders = Arc::new(Mutex::new(HashMap::new()));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                let senders = senders.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, state, senders) {
                        log::error!("Client handler error: {}", e);
                    }
                });
            }
            Err(e) => {
                log::error!("Connection error: {}", e);
            }
        }
    }

    Ok(())
}

fn handle_client(
    stream: TcpStream,
    state: SharedState,
    senders: ClientSenders,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ws = accept(stream)?;

    let client_id = {
        let mut s = state.lock().unwrap();
        let id = s.next_id;
        s.next_id += 1;
        id
    };

    log::info!("Client {} connected", client_id);

    // Create channel for sending messages to this client
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    senders.lock().unwrap().insert(client_id, tx);

    // Set non-blocking for polling
    ws.get_ref().set_nonblocking(true)?;

    loop {
        // Check for incoming WebSocket messages
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(msg) = serde_json::from_str::<SignalMessage>(&text) {
                    handle_message(client_id, msg, &state, &senders);
                }
            }
            Ok(Message::Close(_)) => {
                break;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                break;
            }
        }

        // Check for outgoing messages
        while let Ok(msg) = rx.try_recv() {
            if ws.write(Message::Text(msg.into())).is_err() {
                break;
            }
        }

        thread::sleep(std::time::Duration::from_millis(10));
    }

    // Cleanup on disconnect
    log::info!("Client {} disconnected", client_id);
    cleanup_client(client_id, &state, &senders);

    Ok(())
}

fn handle_message(
    client_id: ClientId,
    msg: SignalMessage,
    state: &SharedState,
    senders: &ClientSenders,
) {
    log::info!("Client {} sent: {}", client_id, msg.msg_type);

    match msg.msg_type.as_str() {
        "join" => {
            let mut s = state.lock().unwrap();

            if let Some(waiting_id) = s.waiting_client.take() {
                // Pair the clients
                s.peers.insert(client_id, waiting_id);
                s.peers.insert(waiting_id, client_id);
                drop(s);

                log::info!("Paired clients {} and {}", client_id, waiting_id);

                send_to_client(waiting_id, r#"{"type":"create-offer"}"#, senders);
                send_to_client(client_id, r#"{"type":"waiting-for-offer"}"#, senders);
            } else {
                s.waiting_client = Some(client_id);
                drop(s);

                log::info!("Client {} waiting for peer", client_id);
                send_to_client(client_id, r#"{"type":"waiting"}"#, senders);
            }
        }
        "offer" | "answer" | "ice-candidate" => {
            let peer_id = {
                let s = state.lock().unwrap();
                s.peers.get(&client_id).copied()
            };

            if let Some(peer_id) = peer_id
                && let Ok(json) = serde_json::to_string(&msg)
            {
                send_to_client(peer_id, &json, senders);
            }
        }
        _ => {}
    }
}

fn send_to_client(client_id: ClientId, msg: &str, senders: &ClientSenders) {
    if let Some(tx) = senders.lock().unwrap().get(&client_id) {
        let _ = tx.send(msg.to_string());
    }
}

fn cleanup_client(client_id: ClientId, state: &SharedState, senders: &ClientSenders) {
    senders.lock().unwrap().remove(&client_id);

    let mut s = state.lock().unwrap();

    if s.waiting_client == Some(client_id) {
        s.waiting_client = None;
    }

    if let Some(peer_id) = s.peers.remove(&client_id) {
        s.peers.remove(&peer_id);
        drop(s);

        send_to_client(peer_id, r#"{"type":"peer-disconnected"}"#, senders);
    }
}
