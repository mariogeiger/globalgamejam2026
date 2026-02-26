# Stage 1: Build WASM client + native server
FROM rust:1.93 AS builder

RUN rustup target add wasm32-unknown-unknown \
 && cargo install trunk

WORKDIR /app
COPY . .

# Build WASM client
RUN cd client && trunk build --release

# Build native server (needs .git for GIT_HASH in build.rs)
RUN cargo build -p server --release

# Stage 2: Minimal runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/client/dist/ client/dist/
COPY --from=builder /app/target/release/server .

ENV RUST_LOG=info
EXPOSE 9000
EXPOSE 3478/udp

CMD ["./server"]
