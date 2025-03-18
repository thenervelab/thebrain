# Bittensor Pallet

## Overview

The Bittensor Pallet is a specialized Substrate FRAME pallet designed to manage weight calculations and submissions for a decentralized machine learning network. This pallet is responsible for periodically submitting weights to the Bittensor chain by calculating weights for each node. Additionally, it contributes to calculating node rankings on the network, ensuring accurate evaluation of node performance metrics.

## Key Features

- **Automated Weight Calculation**: 
  - Calculates weights for network miners based on various performance metrics
  - Supports geographically distributed node evaluation

- **Offchain Worker Integration**:
  - Periodically submits node weights to the network
  - Runs only for validator nodes

- **Version Key Management**:
  - Fetches and manages version keys for network synchronization

## Core Functions

### Weight Calculation
- Evaluates miners based on:
  - Node metrics
  - Geographical distribution
  - Performance characteristics

### Offchain Submission
- Periodic weight submission for network consensus
- Supports RPC-based extrinsic submission

## Configuration Parameters

- `FinneyRpcUrl`: RPC endpoint for network communication
- `VersionKeyStorageKey`: Storage key for version management
- `NetUid`: Unique network identifier
- `BittensorCallSubmission`: Frequency of weight submissions

## Prerequisites

- Substrate blockchain framework
- Registered network nodes
- Execution unit metrics collection
