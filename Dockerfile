# Build stage (same for both)
FROM ubuntu:24.04 AS builder

RUN apt-get update && apt-get install -y \
    git curl build-essential cmake clang pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
ENV PROTOC=/usr/bin/protoc

WORKDIR /hippius
COPY . .
RUN cargo build --release --bin hippius

# Runtime stage (supports both miner/validator)
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl-dev libgcc-s1 libstdc++6 curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /hippius/target/release/hippius /usr/local/bin/

RUN useradd -m -u 5000 -U -s /bin/sh -d /hippius hippius && \
    mkdir -p /data && \
    chown -R hippius:hippius /data

USER hippius

EXPOSE 30333 9933 9944 9615
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=3s CMD curl --fail http://localhost:9933/health || exit 1

# Default to miner mode, but allow override via CMD
ENTRYPOINT ["/usr/local/bin/hippius"]
CMD ["--base-path=/data", \
     "--chain=hippius", \
     "--offchain-worker=Always", \
     "--rpc-external", \
     "--unsafe-rpc-external", \
     "--rpc-cors=all", \
     "--database=paritydb", \
     "--name=hippius-node", \
     "--telemetry-url=wss://telemetry.polkadot.io/submit/ 0", \
     "--no-mdns", \
     "--out-peers=450", \
     "--in-peers=525"]