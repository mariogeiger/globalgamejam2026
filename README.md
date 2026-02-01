# Global Game Jam 2026

A multiplayer free-for-all FPS game built with Rust and WebGPU, running entirely in the browser via WebAssembly.

## About

This game was created for [Global Game Jam 2026](https://globalgamejam.org/). It's a browser-based first-person shooter where players compete in a last-man-standing deathmatch.

**Gameplay:**
- Players spawn on the map during a **grace period** (10 seconds) where no damage is dealt
- After the grace period, it's everyone for themselves
- **Eliminate enemies by staring at them** - keep an opponent in your crosshair for 1 second to kill them
- Be the last one standing to win
- The winner is celebrated with a victory screen before the game restarts

**Tech Stack:**
- **Rust** - Game logic and server
- **WebGPU/wgpu** - Modern GPU rendering
- **WebAssembly** - Runs natively in browser
- **WebRTC** - Peer-to-peer multiplayer
- **glTF/GLB** - 3D model format for maps and characters

## Architecture

```
┌──────────────────────────────────────────────────┐
│           Native Rust Server Binary              │
├──────────────────────────────────────────────────┤
│  HTTP Server (tiny_http)         :8080           │
│  └── Serves dist/ folder (index.html, WASM, JS)  │
├──────────────────────────────────────────────────┤
│  WebSocket Server (tungstenite)  :9000           │
│  └── Signaling: pairs clients, forwards SDP/ICE  │
├──────────────────────────────────────────────────┤
│  STUN Server (UDP)               :3478           │
│  └── NAT traversal for WebRTC                    │
└──────────────────────────────────────────────────┘
```

Everything runs from a single Rust codebase organized as a Cargo workspace:

```
globalgamejam2026/
├── Cargo.toml              # Workspace root
├── client/                 # WASM game client
│   ├── .cargo/config.toml  # Sets wasm32 as default target
│   ├── Cargo.toml
│   ├── index.html
│   ├── assets/
│   └── src/
└── server/                 # Native Rust server
    ├── Cargo.toml
    └── src/
        ├── main.rs         # HTTP server + spawns WS/STUN threads
        ├── signaling.rs    # WebSocket client pairing & message relay
        └── stun.rs         # STUN protocol implementation (RFC 5389)
```

## Requirements

- [Rust](https://rustup.rs/)
- [Trunk](https://trunkrs.dev/) - `cargo install trunk`
- WASM target - `rustup target add wasm32-unknown-unknown`

## Development

Run the client and server in separate terminals:

```bash
# Terminal 1: WASM client with hot-reload
cd client && trunk serve

# Terminal 2: Native server
cargo run -p server
```

Then open http://localhost:8080

## Production

```bash
# Build WASM client → client/dist/
cd client && trunk build --release && cd ..

# Build native server
cargo build -p server --release

# Run server (serves static files from client/dist/)
./target/release/server
```

Then open http://localhost:8080

> **Note:** WebGPU requires a secure context. Access via `localhost` works, but LAN IPs (e.g. `192.168.x.x`) require HTTPS.

## Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 8080 | HTTP | Static files (index.html, WASM, JS, assets) |
| 9000 | WebSocket | Signaling server for WebRTC peer pairing |
| 3478 | UDP | STUN server for NAT traversal |

## Development Setup

Enable the pre-commit hook for automatic code formatting:

```bash
git config core.hooksPath .githooks
```

This runs `cargo fmt` before each commit.

## Controls

- **WASD** - Move
- **Mouse** - Look around
- **Space** - Jump

## Multiplayer

Players connect via WebRTC for low-latency peer-to-peer gameplay:

1. Players connect to the WebSocket signaling server
2. The server pairs players and facilitates SDP offer/answer exchange
3. STUN server helps with NAT traversal
4. Once connected, players communicate directly via WebRTC DataChannel
5. Game waits for players, then starts the grace period countdown

Late joiners wait for the current round to finish before joining the next one.
