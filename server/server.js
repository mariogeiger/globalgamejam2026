import { WebSocketServer } from 'ws';

const PORT = process.env.PORT || 9000;
const wss = new WebSocketServer({ host: '0.0.0.0', port: PORT });

let waitingClient = null;

console.log(`Signaling server running on port ${PORT}`);

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
