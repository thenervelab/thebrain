# Registration Pallet

## Overview

The Registration Pallet is a critical component of the blockchain infrastructure designed to manage and track node registrations within the network. It provides a robust mechanism for registering different types of nodes, tracking their status, and ensuring network integrity.

## Features

- Node Registration: Allows registration of different node types (e.g., Miners)
- Node Status Management: Track and update node status (Online, Degraded)
- Staking Verification: Ensure nodes meet minimum staking requirements
- Secure Registration Process: Supports both signed and unsigned transactions

## Extrinsics

### 1. `register_node`
- Registers a new node in the network
- Requires:
  - Node Type
  - Node ID
  - IPFS Node ID (for Miners)
- Prevents duplicate registrations
- Emits `NodeRegistered` event

### 2. `set_node_status_to_degraded`
- Root-only function to mark a node as degraded
- Requires:
  - Node ID
- Only applicable to Miner nodes
- Emits `NodeStatusUpdated` event

### 3. `set_node_status_to_degraded_unsigned`
- Unsigned transaction for marking a node as degraded
- Requires:
  - Node ID
  - Signature
- Provides an alternative method for status updates

### 4. `force_register_node`
- Root-only function for force registering a node
- Bypasses standard registration checks
- Useful for administrative purposes

## Helper Functions

- `get_all_active_miners()`: Retrieves all active miners
- `get_all_miners_with_min_staked()`: Finds miners meeting minimum stake requirements
- `get_registered_node()`: Retrieves information for a specific node

## Configuration

Requires configuration with:
- Minimum Miner Stake Threshold
- Chain Decimals
- Event and System Configuration

## Error Handling

Comprehensive error handling for scenarios like:
- Duplicate registrations
- Invalid node types
- Insufficient staking
- Missing required information

## Security Features

- Signature validation
- Root-only administrative functions
- Offchain unsigned transaction support
- Storage locks to prevent concurrent modifications

## Events

- `NodeRegistered`: Emitted when a new node is registered
- `NodeStatusUpdated`: Emitted when a node's status changes

## Dependencies

- `frame_support`
- `frame_system`
- `pallet_balances`
- `pallet_staking`
- `pallet_utils`