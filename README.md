# Global Game Jam 2026

Multiplayer FPS game built with Rust and WebGPU, running in the browser via WebAssembly.

## Requirements

- [Rust](https://rustup.rs/)
- [Trunk](https://trunkrs.dev/) - `cargo install trunk`
- WASM target - `rustup target add wasm32-unknown-unknown`

## Running

```bash
trunk serve
```

Then open http://localhost:8080

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

Two players connect via WebRTC. First player joins Team A (blue), second joins Team B (red).
