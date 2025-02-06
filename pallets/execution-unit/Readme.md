# Execution Unit Pallet

## Overview

The Execution Unit Pallet is a critical component designed to manage and track node performance, hardware specifications, and system metrics in a decentralized network. It provides a robust mechanism for collecting, storing, and validating node-related information through offchain workers.

## Key Features

- **Node Metrics Tracking**: Comprehensive collection of hardware and performance metrics
- **Offchain Worker Integration**: Periodic collection of system information
- **Performance Benchmarking**: Ability to assess node capabilities
- **Secure Unsigned Transactions**: Safe submission of node metrics

## Extrinsics

The pallet provides several key extrinsics:

1. [put_node_specs](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:371:2-461:3): Store hardware specifications for a node
   - Captures system information like CPU, memory, storage, and network details
   - Supports different node types (Validator, Miner)

2. [update_metrics_data](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:491:2-549:3): Update comprehensive node performance metrics
   - Track latency, peer count, challenges, and uptime
   - Calculates performance scores and reliability metrics

3. [update_pin_check_metrics](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:464:2-488:3): Update IPFS pin checking metrics
   - Track total and successful pin checks
   - Helps in assessing storage reliability

4. [update_block_time](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:551:2-575:3): Record block processing times
   - Helps in tracking node performance over time

5. [store_offline_status_of_miner](cci:1://file:///home/faiz/Documents/GitHub/thebrain/pallets/execution-unit/src/lib.rs:577:2-606:3): Record offline status for miners
   - Tracks node availability and reliability

## Design Principles

- **Security**: Uses signed payloads and unsigned transactions
- **Flexibility**: Supports various node types and metrics
- **Performance**: Lightweight offchain worker design
- **Extensibility**: Easy to add new metrics and tracking mechanisms

## Dependencies

- Substrate Frame
- Pallet Registration
- IPFS Pin Pallet
- Metagraph Pallet
