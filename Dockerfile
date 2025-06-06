FROM ubuntu:24.04

# Install build and runtime dependencies
RUN apt-get update && apt-get install -y \
    git curl build-essential cmake clang pkg-config libssl-dev protobuf-compiler \
    ca-certificates libgcc-s1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
ENV PROTOC=/usr/bin/protoc

# Build Hippius
WORKDIR /hippius
COPY . .
RUN cargo build --release --bin hippius
RUN mv /hippius/target/release/hippius /usr/local/bin/ && rm -rf /hippius

# Set up non-root user and data directory
RUN useradd -m -u 5000 -U -s /bin/sh -d /hippius hippius && \
    mkdir -p /data

# Create entrypoint script in /usr/local/bin instead of /data
USER root
RUN echo '#!/bin/sh\nset -e\nCMD_ARGS="--offchain-worker=Always --base-path=${BASE_PATH} --chain=${CHAIN} --port=30333 --unsafe-rpc-external --rpc-cors=all --rpc-external --database=paritydb --name=hippius-storage-miner --no-mdns --out-peers=450 --in-peers=525 --bootnodes=/ip4/149.5.28.23/tcp/30333/p2p/12D3KooWMuNG6ASCMDsyA45sUgYsYs1qHHrhkfhaMx7QNF98aWMZ --node-key-file=${NODE_KEY_FILE}"\nif [ -n "$VALIDATOR" ]; then\n  CMD_ARGS="$CMD_ARGS --validator --rpc-methods=Unsafe "\nfi\nexec /usr/local/bin/hippius $CMD_ARGS' > /usr/local/bin/entrypoint.sh && \
    chmod +x /usr/local/bin/entrypoint.sh

USER hippius
WORKDIR /data

EXPOSE 30333 9933 9944 9615
VOLUME ["/data"]

# Environment variables for runtime arguments
ENV BASE_PATH="/data"
ENV CHAIN="dev"
ENV NODE_KEY_FILE="/data/node-key"
ENV VALIDATOR=""

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]