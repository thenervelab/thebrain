# Hippius Network: The Next-Generation Decentralized Infrastructure

## 🌐 Project Overview

Hippius Network is an advanced blockchain platform built on Substrate, designed to revolutionize decentralized computing with innovative reward mechanisms, robust governance, and multi-dimensional value creation.

## 🔑 Key Features

### 💡 Unique Reward Distribution Model

#### Reward Allocation Breakdown
- **Validators (Staking pallet validators)**: 75% of Marketplace balance
- **Treasury / Pot**: 25% of Marketplace balance (sent to the configured treasury/pot address)

#### Validator Reward Mechanism
1. **Era-Based Distribution**
   - Rewards calculated and distributed every 6-hour era
   - Proportional allocation based on validator performance and stake

2. **Reward Calculation**
   - Base reward from marketplace economic activity
   - Performance-based multipliers
   - Stake-weighted distribution

### 🏦 Economic Model
- **Minimum Validator Stake**: Configurable
- **Maximum Active Validators**: 1000
- **Nomination Mechanism**: Flexible staking with up to 16 nominations per account

### 🔒 Consensus & Security
- **Consensus Algorithm**: Proof-of-Stake (PoS)
- **Block Finality**: GRANDPA Finality Gadget
- **Validator Selection**: Adaptive NPoS (Nominated Proof-of-Stake)

## 🚀 Technical Specifications

### Network Parameters
- **Network Type**: Substrate-based Blockchain
- **Token Decimals**: 18
- **Era Duration**: 6 hours
- **Bonding Period**: 28 days
- **Unbonding Period**: 28 days
- **Slash Defer Duration**: 27 days

### Validator Requirements
1. **Minimum Stake**
   - Initial Validator Stake: Configurable
   - Recommended Minimum: [Specify Exact Amount]

2. **Performance Metrics**
   - Uptime Tracking
   - Slashing for Malicious Behavior
   - Rewards Proportional to Consistent Performance

## 💰 Economic Incentives

### Reward Distribution Workflow

**Era payout (mainnet runtime — `MarketplaceRewardPayout`):**

Funds taken from the **marketplace pallet account** each era:

- **75%** → **active validators** (native balance, split **equally** across the session validator set)
- **25%** → **treasury** (hardcoded recipient in runtime; see `MarketplaceRewardPayout::era_payout`)

This is **not** a 70/20/10 split; the live logic is **75/25** (validators vs treasury).

**Alpha released inside the marketplace pallet (`distribute_alpha`):**

When alpha is routed through `pallets/marketplace`’s `distribute_alpha`:

- **75%** → **rankings** pallet account  
- **25%** → **marketplace** pallet account  

That split is **hardcoded** in the marketplace pallet (not configurable via runtime constants today). It is a **different** flow from the era marketplace → validators/treasury payout above.

**Registration pallet** and other accounts may hold separate balances for fees; read `era_payout` and marketplace call sites for the full picture.

### Validator Reward Calculation
1. **Base Reward Calculation**
   - Total Era Revenue
   - Number of Active Validators
   - Individual Validator Performance

2. **Reward Components**
   - Base Stake Reward
   - Performance Bonus
   - Consistency Multiplier

### 🛡️ Risk Management
- **Slashing Mechanism**
  - Penalties for Validator Misbehavior
  - Progressive Slashing Rates
- **Minimum Performance Threshold**

## 📦 Installation & Setup

### Prerequisites
- Rust (latest stable)
- Substrate Development Environment
- Minimum Hardware:
  - CPU: 4 cores
  - RAM: 16GB
  - Storage: 500GB SSD
  - Bandwidth: 100 Mbps


### Quick Start
```bash
# Clone Repository
git clone https://github.com/thenervelab/thebrain.git
cd thebrain

# Install Dependencies
rustup update
rustup component add rust-src
cargo build --release

# Start Node
./target/release/hippius-node 
```


### 🐳 Docker Quick Start

> **Note:**  
> - Ensure the `hippius-node` binary is present in the project root **before** building any Docker image.  
> - Double-check that the keystore path and network file path are correct for your setup **before running** the container.


## Pull Image from GitHub Container Registry

You can pull the pre-built hippius-node image directly from GitHub Container Registry (GHCR):

### Pull the latest image
docker pull ghcr.io/thenervelab/thebrain/hippius-node:latest

### Optionally, pull a specific version using the SHA tag (replace <SHA> with the desired tag)
docker pull ghcr.io/thenervelab/thebrain/hippius-node:<SHA>

## Build and run Image Locally

```bash
# Build Validator Docker Image
docker build -t hippius-node .

# Set permissions for data directory
sudo chown -R $USER:$USER /opt/hippius/data
chmod -R 777 /opt/hippius/data
```

### Run as Validator Node

```bash

# Run Validator Node in Docker
docker run -d --name hippius-validator \
  --user 1000:1000 \
  -p 30333:30333 -p 9933:9933 -p 9944:9944 -p 9615:9615 \
  -v /opt/hippius/data:/data \
  -v /opt/hippius/data/chains/hippius-testnet/network/secret_ed25519:/data/node-key \
  hippius-node \
  --validator \                       
  --rpc-methods=Unsafe \              
  --name="hippius-testnet-validator" \   
  --bootnodes=/ip4/91.134.72.142/tcp/30333/ws/p2p/12D3KooWRJdyfLdhPzyQrUHKWdEooNPsNFWRTfCS8tDSeysPDxVR
```

#### Miner Node

```bash
# Build Miner Docker Image
docker build -t hippius-miner .

# Set permissions for data directory
sudo chown -R $USER:$USER /opt/hippius/data
chmod -R 777 /opt/hippius/data

# Run Miner Node in Docker
docker run -d --name hippius-miner \
  --user 1000:1000 \
  -p 30333:30333 -p 9933:9933 -p 9944:9944 -p 9615:9615 \
  -v /opt/hippius/data:/data \
  -v /opt/hippius/data/chains/hippius-testnet/network/secret_ed25519:/data/node-key \
  hippius-node
```


get all users for arion

get size of each user

clear users data 