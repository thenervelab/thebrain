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

### 10. `get_user_vms`

#### Description
Retrieves all virtual machines (VMs) owned by a specific account, including their current status, deployment details, and associated miner information.

#### Parameters
- `account` (AccountId32): The account to retrieve VM details for.

#### Response
Returns a list of `UserVmDetails` objects, each containing:
- `request_id`: Unique identifier for the VM request (u128).
- `status`: Current status of the VM, which can be:
  - `Pending`
  - `Stopped`
  - `InProgress`
  - `Running`
  - `Failed`
  - `Cancelled`
- `plan_id`: Identifier for the compute plan.
- `created_at`: Timestamp of VM request creation.
- `miner_node_id`: Optional ID of the miner hosting the VM.
- `miner_account_id`: Optional account ID of the miner.
- `hypervisor_ip`: Optional IP address of the hypervisor.
- `vnc_port`: Optional VNC port for accessing the VM.
- `ip_assigned`: Optional IP address assigned to the VM.
- `error`: Optional error message if the VM deployment failed.
- `is_fulfilled`: Boolean indicating if the VM request has been fulfilled.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_user_vms",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

#### Notes
- Returns detailed information about VMs owned by the account.
- Useful for tracking VM deployment and management across the network.

### 11. `get_client_ip`

#### Description
Retrieves the IP address assigned to a specific client account.

#### Parameters
- `client_id` (AccountId32): The account ID of the client to retrieve the IP for

#### Response
Returns an optional IP address as a `Vec<u8>`:
- If an IP is assigned to the client, returns the IP address
- If no IP is assigned, returns `null`

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_client_ip",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

#### Example Responses
```json
{
    "jsonrpc": "2.0",
    "result": "10.0.16.42",
    "id": 1
}
```
or 
```json
{
    "jsonrpc": "2.0",
    "result": null,
    "id": 1
}
```

#### Notes
- Returns the IP address assigned to a client during account creation or IP allocation
- Useful for retrieving a client's network-assigned IP
- Part of the IP management system in the Hippius network

### 12. `get_hypervisor_ip`

#### Description
Retrieves the IP address assigned to a specific hypervisor.

#### Parameters
- `hypervisor_id` (Vec<u8>): The unique identifier of the hypervisor

#### Response
Returns an optional IP address as a `Vec<u8>`:
- If an IP is assigned to the hypervisor, returns the IP address
- If no IP is assigned, returns `null`

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_hypervisor_ip",
    "params": ["hypervisor_unique_id"],
    "id": 1
}
```

#### Example Responses
```json
{
    "jsonrpc": "2.0",
    "result": "10.0.1.42",
    "id": 1
}
```
or 
```json
{
    "jsonrpc": "2.0",
    "result": null,
    "id": 1
}
```

### 13. `get_vm_ip`

#### Description
Retrieves the IP address assigned to a specific virtual machine (VM).

#### Parameters
- `vm_id` (Vec<u8>): The unique identifier of the VM

#### Response
Returns an optional IP address as a `Vec<u8>`:
- If an IP is assigned to the VM, returns the IP address
- If no IP is assigned, returns `null`

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_vm_ip",
    "params": ["vm_unique_id"],
    "id": 1
}
```

#### Example Responses
```json
{
    "jsonrpc": "2.0",
    "result": "10.0.80.42",
    "id": 1
}
```
or 
```json
{
    "jsonrpc": "2.0",
    "result": null,
    "id": 1
}
```

### 14. `get_storage_miner_ip`

#### Description
Retrieves the IP address assigned to a specific storage miner.

#### Parameters
- `miner_id` (Vec<u8>): The unique identifier of the storage miner

#### Response
Returns an optional IP address as a `Vec<u8>`:
- If an IP is assigned to the storage miner, returns the IP address
- If no IP is assigned, returns `null`

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_storage_miner_ip",
    "params": ["miner_unique_id"],
    "id": 1
}
```

#### Example Responses
```json
{
    "jsonrpc": "2.0",
    "result": "10.0.128.42",
    "id": 1
}
```
or 
```json
{
    "jsonrpc": "2.0",
    "result": null,
    "id": 1
}
```

### 15. `get_bucket_size`

#### Description
Retrieves the size of a specific bucket.

#### Parameters
- [bucket_name](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/storage-s3/src/lib.rs:111:8-133:9) (Vec<u8>): The name of the bucket to retrieve the size for.

#### Response
Returns the size of the bucket as a `u128` value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_bucket_size",
    "params": [["bucket_name"]],
    "id": 1
}
```


### 16. `get_user_bandwidth`

#### Description
Retrieves the bandwidth size for a specific user.

#### Parameters
- `account_id` (AccountId32): The account ID of the user to retrieve bandwidth for.

#### Response
Returns the bandwidth size as a `u128` value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_user_bandwidth",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```


### 17. `get_total_bucket_size`

#### Description
Retrieves the total size of all buckets for a specific user.

#### Parameters
- `account_id` (AccountId32): The account ID of the user to retrieve the total bucket size for.

#### Response
Returns the total bucket size as a `u128` value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_total_bucket_size",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 18. `get_miner_info`

#### Description
Retrieves information about a miner associated with a specific account ID.

#### Parameters
- `account_id` (AccountId32): The account ID of the miner to retrieve information for.

#### Response
Returns an optional tuple containing:
- `NodeType`: The type of the miner (e.g., Validator, StorageMiner).
- `Status`: The current status of the miner (e.g., Online, Degraded, Offline).

If no miner is found for the provided account ID, the response will be `null`.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_miner_info",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 19. `get_free_credits_rpc`

#### Description
Retrieves the free credits for a specific account or all accounts if none is provided.

#### Parameters
- `account` (Option<AccountId32>): The account to check free credits for. If `None`, returns free credits for all accounts.

#### Response
Returns a list of tuples containing:
- `AccountId32`: The account ID.
- `u128`: The amount of free credits.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_free_credits_rpc",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```
### 20. `get_referred_users`

#### Description
Retrieves all users referred by a specific account.

#### Parameters
- `account` (AccountId32): The account to check referrals for.

#### Response
Returns a list of tuples containing:
- Returns a list of AccountId32 objects representing the referred users.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_referred_users",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 21. `get_referral_rewards`

#### Description
Retrieves total referral rewards earned by a specific account.

#### Parameters
- `account` (AccountId32): The account to check referral rewards for.

#### Response
- Returns the total rewards as a u128 value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_referral_rewards",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 22. `total_referral_codes`

#### Description
Retrieves the total number of referral codes created in the network.

#### Response
- Returns the total count as a u32 value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "total_referral_codes",
    "params": [],
    "id": 1
}
```

### 23. `total_referral_rewards`

#### Description
Retrieves the total referral rewards distributed across the network

#### Response
- Returns the total rewards as a u128 value.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "total_referral_rewards",
    "params": [],
    "id": 1
}
```

### 24. `get_referral_codes`

#### Description
Retrieves all referral codes owned by a specific account.

#### Parameters
- `account` (AccountId32): The account to check referral codes for.

#### Response
- Returns a list of Vec<u8> representing the referral codes.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_referral_codes",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```

### 25. `get_batches_for_user`

#### Description
Retrieves all batches associated with a specific user account.

#### Parameters
- `account_id` (AccountId32): The account ID of the user to retrieve batches for.

#### Response
Returns a list of `Batch<AccountId32, u32>` objects, each containing:
- `owner`: The account ID of the owner.
- `credit_amount`: The total credit amount associated with the batch.
- `alpha_amount`: The total alpha amount associated with the batch.
- `remaining_credits`: The remaining credits in the batch.
- `remaining_alpha`: The remaining alpha in the batch.
- `pending_alpha`: The pending alpha in the batch.
- `is_frozen`: Indicates whether the batch is frozen.
- `release_time`: The time at which the batch will be released.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_batches_for_user",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```


### 26. `get_batch_by_id`

#### Description
Retrieves a specific batch by its unique ID.

#### Parameters
- `batch_id` (u64): The unique identifier of the batch to retrieve.

#### Response
Returns an optional `Batch<AccountId32, u32>` object containing:
- `owner`: The account ID of the owner.
- `credit_amount`: The total credit amount associated with the batch.
- `alpha_amount`: The total alpha amount associated with the batch.
- `remaining_credits`: The remaining credits in the batch.
- `remaining_alpha`: The remaining alpha in the batch.
- `pending_alpha`: The pending alpha in the batch.
- `is_frozen`: Indicates whether the batch is frozen.
- `release_time`: The time at which the batch will be released.

If no batch is found for the provided ID, the response will be `null`.

#### Example Request
```json
{
    "jsonrpc": "2.0",
    "method": "get_batches_for_user",
    "params": ["5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKv3gB"],
    "id": 1
}
```


#### Notes
- These methods are part of the IP management system in the Hippius network
- Useful for retrieving network-assigned IP addresses for different node types
- Returns `null` if no IP has been assigned to the specified node

### Notes
- All methods return `RpcResult`, which handles potential errors
- Ensure proper authentication and authorization when using these RPC methods
- Reward values are represented in the network's base currency unit
- The returned value represents the total bytes of unique files owned by the account
- Only files with approved storage requests are included
- Duplicate file hashes are counted only once