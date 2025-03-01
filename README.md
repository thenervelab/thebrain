# Hippius - Decentralized Infrastructure Subnet

A comprehensive Bittensor subnet providing distributed storage, compute, and blockchain management capabilities.

## Overview

Hippius is a Bittensor subnet that enables decentralized file storage and computation across the network. It leverages Bittensor's incentive mechanism while using a network of miners and validators to ensure secure and reliable data management.

## Core Features

- Decentralized file storage with IPFS integration
- Reputation-based worker node management
- Marketplace for storage and compute resources
- Advanced authorization system with address linking
- Distributed offchain workers for real-time system metrics

## Node Architecture

Each network participant runs specialized components and offchain workers that enable decentralized operations:

- **Storage Miners**: 
  - Run IPFS nodes for distributed file storage
  - Manage dedicated storage nodes and disk infrastructure
  - Monitor storage capacity, file health, and replication status via offchain workers

- **Compute Miners**: 
  - Operate as hypervisors for virtualized compute resources
  - Manage task execution and resource allocation
  - Track resource availability and performance metrics via offchain workers

- **Validators**: 
  - Verify network operations and collect performance data
  - Ensure network security and proper resource allocation
  - monitor storage and compute usage via offchain workers
  - manage replication and task assignment via offchain workers



## Security Features

- Validator-miner secure communication protocol
- Blacklist and ban management
- Slashing mechanisms for bad actors
- Reward system for reliable nodes


# Node Metrics RPC API

## Overview
The Node Metrics RPC API provides a comprehensive set of methods to retrieve detailed metrics and reward information for nodes in the Hippius network. These endpoints enable users and developers to access real-time performance and reward data across different node types.

## Available RPC Methods

### 1. `get_active_nodes_metrics_by_type`

#### Description
Retrieves detailed metrics for active nodes of a specified node type.

#### Parameters
- `node_type` (NodeType): The type of node to retrieve metrics for
  - Possible values: 
    - `Validator`
    - `StorageMiner`
    - `StorageS3`
    - `ComputeMiner`
    - `GpuMiner`

#### Response
Returns a list of `NodeMetricsData` objects, which include:
- Miner ID
- Bandwidth usage
- Storage metrics
- Geolocation
- Performance indicators (pin checks, latency, challenges)
- System specifications (CPU, memory, GPU)
- Network and disk information

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_active_nodes_metrics_by_type",
    "params": ["StorageMiner"],
    "id": 1
}
```

### 2. `get_total_node_rewards`

#### Description
Retrieves the total rewards for a specific account across all node types.

#### Parameters
- `account` (AccountId32): The account to check rewards for

#### Response
Returns the total rewards as a `u128` value representing the cumulative rewards.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_total_node_rewards",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 3. `get_total_distributed_rewards_by_node_type`

#### Description
Retrieves the total distributed rewards for a specific node type.

#### Parameters
- `node_type` (NodeType): The node type to check rewards for
  - Possible values: 
    - `Validator`
    - `StorageMiner`
    - `StorageS3`
    - `ComputeMiner`
    - `GpuMiner`

#### Response
Returns the total distributed rewards as a `u128` value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_total_distributed_rewards_by_node_type",
    "params": ["StorageMiner"],
    "id": 1
}
```

### 4. `get_miners_total_rewards`

#### Description
Retrieves total rewards for miners of a specific node type.

#### Parameters
- `node_type` (NodeType): The node type to retrieve miner rewards for

#### Response
Returns a list of `MinerRewardSummary` objects, each containing:
- Account ID
- Total reward amount

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_miners_total_rewards",
    "params": ["ComputeMiner"],
    "id": 1
}
```

### 5. `get_account_pending_rewards`

#### Description
Retrieves pending rewards for a specific account.

#### Parameters
- `account` (AccountId32): The account to check pending rewards for

#### Response
Returns a list of `MinerRewardSummary` objects representing pending rewards.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_account_pending_rewards",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 6. `get_miners_pending_rewards`

#### Description
Retrieves pending rewards for miners of a specific node type.

#### Parameters
- `node_type` (NodeType): The node type to retrieve pending rewards for

#### Response
Returns a list of `MinerRewardSummary` objects representing pending rewards.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_miners_pending_rewards",
    "params": ["StorageS3"],
    "id": 1
}
```

### 7. `calculate_total_file_size`

#### Description
Calculates the total file size for all approved and pinned files owned by a specific account.

#### Parameters
- `account` (AccountId32): The account to calculate total file size for

#### Response
Returns the total file size in bytes as a `u128` value.

#### Behavior
- Considers only approved storage requests
- Counts each unique file hash only once
- Uses the file size of the first pinned request for each unique file hash

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "calculate_total_file_size",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

#### Example Response
```json
{
    "jsonrpc": "2.0",
    "result": 1073741824,
    "id": 1
}
```

### 8. `get_user_files`

#### Description
Retrieves all files owned by a specific account, including their file hashes, names, and the miner node IDs that have pinned these files.

#### Parameters
- `account` (AccountId32): The account to retrieve files for

#### Response
Returns a list of `UserFile` objects, each containing:
- `file_hash`: The unique hash of the file
- `file_name`: The name of the file
- `miner_ids`: A list of miner node IDs that have pinned the file
- `file_size`: The size of the file in bytes

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_user_files",
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
            "file_hash": "0x1234...",
            "file_name": "example.txt",
            "miner_ids": [
                "0xabcd...",
                "0xefgh..."
            ],
            "file_size": 1024
        },
        {
            "file_hash": "0x5678...",
            "file_name": "document.pdf",
            "miner_ids": [
                "0xijkl..."
            ],
            "file_size": 2048
        }
    ],
    "id": 1
}
```

#### Notes
- Returns detailed information about files owned by the account
- Includes the file hash, name, file size, and the list of miner nodes that have pinned the file
- File size is represented in bytes
- Useful for tracking file distribution, replication, and storage usage across the network

### Notes
- All methods return `RpcResult`, which handles potential errors
- Ensure proper authentication and authorization when using these RPC methods
- Reward values are represented in the network's base currency unit
- The returned value represents the total bytes of unique files owned by the account
- Only files with approved storage requests are included
- Duplicate file hashes are counted only once