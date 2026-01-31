import { WebSocketServer } from 'ws';
import dgram from 'dgram';

const WS_PORT = process.env.PORT || 9000;
const STUN_PORT = process.env.STUN_PORT || 3478;

// WebSocket signaling server
const wss = new WebSocketServer({ host: '0.0.0.0', port: WS_PORT });

let waitingClient = null;

console.log(`Signaling server running on port ${WS_PORT}`);

// Minimal STUN server implementation (RFC 5389)
// STUN is simple: client asks "what's my public IP:port?", server responds
const STUN_BINDING_REQUEST = 0x0001;
const STUN_BINDING_RESPONSE = 0x0101;
const STUN_MAGIC_COOKIE = 0x2112A442;
const ATTR_XOR_MAPPED_ADDRESS = 0x0020;

const stunServer = dgram.createSocket('udp4');

stunServer.on('message', (msg, rinfo) => {
    // Validate minimum STUN header (20 bytes)
    if (msg.length < 20) return;
    
    const msgType = msg.readUInt16BE(0);
    const msgLength = msg.readUInt16BE(2);
    const magicCookie = msg.readUInt32BE(4);
    
    // Check if it's a STUN Binding Request
    if (msgType !== STUN_BINDING_REQUEST || magicCookie !== STUN_MAGIC_COOKIE) return;
    
    // Extract transaction ID (12 bytes at offset 8)
    const transactionId = msg.slice(8, 20);
    
    // Build STUN Binding Response with XOR-MAPPED-ADDRESS
    const response = Buffer.alloc(32);
    
    // Header
    response.writeUInt16BE(STUN_BINDING_RESPONSE, 0);  // Message type
    response.writeUInt16BE(12, 2);                      // Message length (excluding header)
    response.writeUInt32BE(STUN_MAGIC_COOKIE, 4);      // Magic cookie
    transactionId.copy(response, 8);                    // Transaction ID
    
    // XOR-MAPPED-ADDRESS attribute
    response.writeUInt16BE(ATTR_XOR_MAPPED_ADDRESS, 20);  // Attribute type
    response.writeUInt16BE(8, 22);                         // Attribute length
    response.writeUInt8(0, 24);                            // Reserved
    response.writeUInt8(0x01, 25);                         // Family (IPv4)
    
    // XOR port with magic cookie upper 16 bits
    const xorPort = rinfo.port ^ (STUN_MAGIC_COOKIE >>> 16);
    response.writeUInt16BE(xorPort, 26);
    
    // XOR IP address with magic cookie
    const ipParts = rinfo.address.split('.').map(Number);
    const ipInt = (ipParts[0] << 24) | (ipParts[1] << 16) | (ipParts[2] << 8) | ipParts[3];
    const xorIp = ipInt ^ STUN_MAGIC_COOKIE;
    response.writeUInt32BE(xorIp >>> 0, 28);
    
    stunServer.send(response, rinfo.port, rinfo.address);
});

stunServer.on('error', (err) => {
    console.error('STUN server error:', err);
});

stunServer.bind(STUN_PORT, '0.0.0.0', () => {
    console.log(`STUN server running on UDP port ${STUN_PORT}`);
});

wss.on('connection', (ws) => {
    console.log('Client connected');

    ws.on('message', (data) => {
        try {
            const msg = JSON.parse(data.toString());
            console.log('Received:', msg.type);

            switch (msg.type) {
                case 'join':
                    if (waitingClient && waitingClient.readyState === 1) {
                        ws.peer = waitingClient;
                        waitingClient.peer = ws;
                        waitingClient.send(JSON.stringify({ type: 'create-offer' }));
                        ws.send(JSON.stringify({ type: 'waiting-for-offer' }));
                        console.log('Paired two clients');
                        waitingClient = null;
                    } else {
                        waitingClient = ws;
                        ws.send(JSON.stringify({ type: 'waiting' }));
                        console.log('Client waiting for peer');
                    }
                    break;

                case 'offer':
                case 'answer':
                case 'ice-candidate':
                    if (ws.peer && ws.peer.readyState === 1) {
                        ws.peer.send(JSON.stringify(msg));
                    }
                    break;
            }
        } catch (e) {
            console.error('Error processing message:', e);
        }
    });

    ws.on('close', () => {
        console.log('Client disconnected');
        if (waitingClient === ws) {
            waitingClient = null;
        }
        if (ws.peer) {
            ws.peer.peer = null;
            try {
                ws.peer.send(JSON.stringify({ type: 'peer-disconnected' }));
            } catch {}
        }
    });
});
