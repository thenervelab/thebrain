# Build stage (same as first Dockerfile)
FROM ubuntu:24.04 AS builder

RUN apt-get update && apt-get install -y \
    git curl build-essential cmake clang pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN apt-get update && apt-get install -y zfsutils-linux pciutils mdadm
RUN apt-get clean && rm -rf /var/lib/apt/lists/*

ENV PATH="/root/.cargo/bin:${PATH}"
ENV PROTOC=/usr/bin/protoc

WORKDIR /hippius
COPY . .
RUN cargo build --release --bin hippius

# Runtime stage (keeping all logic from second Dockerfile)
FROM ubuntu:24.04

# Install only runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl-dev libgcc-s1 libstdc++6 \
    curl jq \
    && rm -rf /var/lib/apt/lists/*

# Copy compiled binary from builder
COPY --from=builder /hippius/target/release/hippius /usr/local/bin/

# Create non-root user (same as original)
RUN useradd -m -u 5000 -U -s /bin/sh -d /hippius hippius && \
    mkdir -p /data

# Update just the entrypoint script section to include node-key-file support
RUN echo '#!/bin/sh\nset -e\nCMD_ARGS="--offchain-worker=Always --base-path=${BASE_PATH} --chain=${CHAIN} --port=30333 --unsafe-rpc-external --rpc-cors=all --rpc-external --rpc-methods=Unsafe --database=paritydb --name=hippius-storage-miner --no-mdns --out-peers=450 --in-peers=525"\n\n# Add bootnodes if specified\nif [ -n "$BOOTNODES" ]; then\n  CMD_ARGS="$CMD_ARGS --bootnodes=${BOOTNODES}"\nfi\n\n# Add validator flag if specified\nif [ -n "$VALIDATOR" ]; then\n  CMD_ARGS="$CMD_ARGS --validator"\nfi\n\n# Add node key file if specified\nif [ -n "$NODE_KEY_FILE" ]; then\n  CMD_ARGS="$CMD_ARGS --node-key-file=${NODE_KEY_FILE}"\nfi\n\nexec /usr/local/bin/hippius $CMD_ARGS' > /usr/local/bin/entrypoint.sh && \
    chmod +x /usr/local/bin/entrypoint.sh

USER hippius
WORKDIR /data

EXPOSE 30333 9933 9944 9615
VOLUME ["/data"]

# Same environment variables
ENV BASE_PATH="/data"
ENV CHAIN="dev"
ENV NODE_KEY_FILE="/data/node-key"
ENV VALIDATOR=""
ENV BOOTNODES=""

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]