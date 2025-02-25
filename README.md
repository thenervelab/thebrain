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
The Node Metrics RPC API provides a way to retrieve metrics for active nodes of different types within the network. This API is particularly useful for monitoring and managing node performance.

## RPC Method: [get_active_nodes_metrics_by_type](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:2004:2-2010:3)

### Description
This method retrieves metrics for active nodes of a specified type. The response includes detailed information about the nodes, such as their performance metrics.

### Endpoint

Example Request:
curl -X POST http://testnet.hippius.com \
-H "Content-Type: application/json" \
-d '{
    "jsonrpc": "2.0",
    "method": "get_active_nodes_metrics_by_type",
    "params": [
        {
            "type": "StorageMiner"
        }
    ],
    "id": 1
}'