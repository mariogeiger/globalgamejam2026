# Global Game Jam 2026

Multiplayer FPS game built with Rust and WebGPU, running in the browser via WebAssembly.

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
# Build WASM client → dist/
cd client && trunk build --release

# Build native server
cargo build -p server --release

# Run server (serves static files from dist/)
./target/release/server
```

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
- **Escape** - Release mouse
- **Click** - Capture mouse

## Multiplayer

Two players connect via WebRTC:

1. Both clients connect to the WebSocket signaling server
2. First player waits, second player triggers pairing
3. Signaling server facilitates SDP offer/answer exchange
4. STUN server helps with NAT traversal
5. Once connected, players communicate peer-to-peer via WebRTC DataChannel

First player joins Team A (blue), second joins Team B (red).
