import { WebSocketServer } from 'ws';
import stun from 'stun';

const WS_PORT = process.env.PORT || 9000;
const STUN_PORT = process.env.STUN_PORT || 3478;

// WebSocket signaling server
const wss = new WebSocketServer({ host: '0.0.0.0', port: WS_PORT });

let waitingClient = null;

console.log(`Signaling server running on port ${WS_PORT}`);

// STUN server (UDP)
const stunServer = stun.createServer({ type: 'udp4' });

stunServer.on('bindingRequest', (request, rinfo) => {
    const response = stun.createMessage(stun.constants.STUN_BINDING_RESPONSE);
    response.setTransactionID(request.transactionId);
    
    // Add XOR-MAPPED-ADDRESS attribute with client's public address
    response.addXorAddress(rinfo.address, rinfo.port);
    
    stunServer.send(response, rinfo.port, rinfo.address);
});

stunServer.listen(STUN_PORT, '0.0.0.0', () => {
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
