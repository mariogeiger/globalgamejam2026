use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::time::Duration;
use turn::Error;
use turn::auth::{AuthHandler, generate_auth_key};
use turn::relay::relay_static::RelayAddressGeneratorStatic;
use turn::server::Server;
use turn::server::config::{ConnConfig, ServerConfig};
use webrtc_util::vnet::net::Net;

/// TURN server credentials - static for simplicity
/// In production, use time-limited credentials
pub const TURN_USERNAME: &str = "ggj26";
pub const TURN_PASSWORD: &str = "globalgamejam2026";
const TURN_REALM: &str = "globalgamejam";

struct StaticAuthHandler {
    cred_map: HashMap<String, Vec<u8>>,
}

impl StaticAuthHandler {
    fn new(username: &str, password: &str, realm: &str) -> Self {
        let mut cred_map = HashMap::new();
        let key = generate_auth_key(username, realm, password);
        cred_map.insert(username.to_owned(), key);
        Self { cred_map }
    }
}

impl AuthHandler for StaticAuthHandler {
    fn auth_handle(
        &self,
        username: &str,
        _realm: &str,
        _src_addr: SocketAddr,
    ) -> Result<Vec<u8>, Error> {
        self.cred_map
            .get(username)
            .cloned()
            .ok_or(Error::ErrFakeErr)
    }
}

pub async fn run_turn_server(port: u16, public_ip: IpAddr) -> Result<(), Error> {
    let conn = Arc::new(UdpSocket::bind(format!("0.0.0.0:{}", port)).await?);
    log::info!(
        "TURN/STUN server running on UDP port {}, public IP: {}",
        port,
        public_ip
    );

    let server = Server::new(ServerConfig {
        conn_configs: vec![ConnConfig {
            conn,
            relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                relay_address: public_ip,
                address: "0.0.0.0".to_owned(),
                net: Arc::new(Net::new(None)),
            }),
        }],
        realm: TURN_REALM.to_owned(),
        auth_handler: Arc::new(StaticAuthHandler::new(
            TURN_USERNAME,
            TURN_PASSWORD,
            TURN_REALM,
        )),
        // Channel bindings expire after 10 minutes (RFC 5766 recommends 10 min)
        channel_bind_timeout: Duration::from_secs(600),
        alloc_close_notify: None,
    })
    .await?;

    log::info!(
        "TURN server ready (credentials: {}:{})",
        TURN_USERNAME,
        TURN_PASSWORD
    );

    // Keep the server running indefinitely
    // The Server handles all STUN/TURN requests internally
    std::future::pending::<()>().await;

    // This code is unreachable but keeps the compiler happy
    server.close().await?;
    Ok(())
}

/// Get public IP from environment variable or use fallback
pub fn get_public_ip() -> IpAddr {
    std::env::var("PUBLIC_IP")
        .ok()
        .and_then(|ip| ip.parse().ok())
        .unwrap_or_else(|| {
            log::warn!("PUBLIC_IP not set, using 0.0.0.0 (TURN relay may not work correctly)");
            "0.0.0.0".parse().unwrap()
        })
}
