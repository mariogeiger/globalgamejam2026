/// STUN Server implementation (RFC 5389)
/// Handles NAT traversal for WebRTC connections
use std::net::UdpSocket;

const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_MAGIC_COOKIE: u32 = 0x2112A442;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

pub fn run_stun_server(port: u16) -> std::io::Result<()> {
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", port))?;
    log::info!("STUN server running on UDP port {}", port);

    let mut buf = [0u8; 512];

    loop {
        let (len, src) = match socket.recv_from(&mut buf) {
            Ok(result) => result,
            Err(e) => {
                log::error!("STUN recv error: {}", e);
                continue;
            }
        };

        if len < 20 {
            continue;
        }

        let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
        let magic_cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);

        if msg_type != STUN_BINDING_REQUEST || magic_cookie != STUN_MAGIC_COOKIE {
            continue;
        }

        // Extract transaction ID (12 bytes at offset 8)
        let mut transaction_id = [0u8; 12];
        transaction_id.copy_from_slice(&buf[8..20]);

        // Build response
        let response = build_binding_response(&transaction_id, src);

        if let Err(e) = socket.send_to(&response, src) {
            log::error!("STUN send error: {}", e);
        }
    }
}

fn build_binding_response(transaction_id: &[u8; 12], src: std::net::SocketAddr) -> [u8; 32] {
    let mut response = [0u8; 32];

    // Header
    response[0..2].copy_from_slice(&STUN_BINDING_RESPONSE.to_be_bytes());
    response[2..4].copy_from_slice(&12u16.to_be_bytes()); // Message length (excluding header)
    response[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    response[8..20].copy_from_slice(transaction_id);

    // XOR-MAPPED-ADDRESS attribute
    response[20..22].copy_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    response[22..24].copy_from_slice(&8u16.to_be_bytes()); // Attribute length
    response[24] = 0; // Reserved
    response[25] = 0x01; // Family (IPv4)

    let port = src.port();
    let xor_port = port ^ ((STUN_MAGIC_COOKIE >> 16) as u16);
    response[26..28].copy_from_slice(&xor_port.to_be_bytes());

    if let std::net::SocketAddr::V4(addr) = src {
        let ip_bytes = addr.ip().octets();
        let ip_int = u32::from_be_bytes(ip_bytes);
        let xor_ip = ip_int ^ STUN_MAGIC_COOKIE;
        response[28..32].copy_from_slice(&xor_ip.to_be_bytes());
    }

    response
}
