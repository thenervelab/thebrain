# Hippius Network: The Next-Generation Decentralized Infrastructure

## 🌐 Project Overview

Hippius Network is an advanced blockchain platform built on Substrate, designed to revolutionize decentralized computing with innovative reward mechanisms, robust governance, and multi-dimensional value creation.

## 🔑 Key Features

### 💡 Unique Reward Distribution Model

#### Reward Allocation Breakdown
- **Staking Rewards**: 20% of Marketplace Revenue
- **Ranking Rewards**: 70% of Marketplace Revenue
  - Split equally between two ranking pallets
- **Treasury**: 10% of Marketplace Revenue

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

Marketplace Revenue (100%) ├── Staking Rewards (20%) │ └── Distributed among Active Validators ├── Ranking Rewards (70%) │ ├── Ranking Pallet 1 (35%) │ └── Ranking Pallet 2 (35%) └── Treasury (10%)



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

## 🌐 Node Metrics RPC API

### 9. `get_user_buckets`

#### Description
Retrieves all buckets owned by a specific account, including their names and sizes.

#### Parameters
- `account` (AccountId32): The account to retrieve buckets for

#### Response
Returns a list of `UserBucket` objects, each containing:
- `bucket_name`: The name of the bucket (as a byte vector)
- `bucket_size`: The size of the bucket (as a vector of u128 values)

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_user_buckets",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

#### Example Response
```json
{
    "jsonrpc": "2.0",
    "result": [
        {
            "bucket_name": "personal_docs",
            "bucket_size": [1024, 2048, 4096]
        },
        {
            "bucket_name": "project_files",
            "bucket_size": [8192, 16384]
        }
    ],
    "id": 1
}
```

### 10. [get_user_vms](cci:1://file:///home/faiz/Documents/GitHub/thebrain/client/rpc/node-metrics/src/lib.rs:125:1-133:2)

#### Description
Retrieves all virtual machines (VMs) owned by a specific account, including their current status, deployment details, and associated miner information.

#### Parameters
- [account](cci:1://file:///home/faiz/Documents/GitHub/thebrain/client/rpc/node-metrics/src/lib.rs:75:1-83:2) (AccountId32): The account to retrieve VM details for

#### Response
Returns a list of [UserVmDetails](cci:2://file:///home/faiz/Documents/GitHub/thebrain/primitives/rpc/node-metrics/src/lib.rs:37:0-49:1) objects, each containing:
- `request_id`: Unique identifier for the VM request (u128)
- `status`: Current status of the VM, which can be:
  - `Pending`
  - `Stopped`
  - `InProgress`
  - `Running`
  - `Failed`
  - `Cancelled`
- `plan_id`: Identifier for the compute plan
- `created_at`: Timestamp of VM request creation
- `miner_node_id`: Optional ID of the miner hosting the VM
- `miner_account_id`: Optional account ID of the miner
- `hypervisor_ip`: Optional IP address of the hypervisor
- `vnc_port`: Optional VNC port for accessing the VM
- `ip_assigned`: Optional IP address assigned to the VM
- `error`: Optional error message if the VM deployment failed
- `is_fulfilled`: Boolean indicating if the VM request has been fulfilled

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_user_vms",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}

#### Notes
- Returns detailed information about buckets owned by the account
- `bucket_name` is a byte vector representing the bucket's identifier
- `bucket_size` is a vector of u128 values, potentially representing different size metrics
- Useful for tracking storage allocation and bucket management across the network

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
git clone https://github.com/your-organization/hippius-node.git
cd hippius-node

# Install Dependencies
rustup update
rustup component add rust-src
cargo build --release

# Start Node
./target/release/hippius-node 
