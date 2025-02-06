# IPFS Pin Pallet

## Overview

The IPFS Pin Pallet is a sophisticated Substrate blockchain component designed to manage decentralized file storage and pinning mechanisms. It provides a robust framework for tracking, storing, and managing file storage requests across a distributed network of miners and validators.

## Key Features

### 1. Decentralized File Storage Management
- Track and manage file storage requests
- Support for multiple file replicas
- Blacklist mechanism for file management

### 2. Miner and Validator Interactions
- Node registration and type verification
- File pinning and unpinning requests
- Offchain worker for continuous file synchronization

### 3. Storage Request Tracking
- Monitor file storage requests
- Track fulfilled and pending storage requests
- Support for subscription-based storage

## Core Functionalities

### File Storage Operations
- `add_ipfs_pin_request()`: Request file storage from miners
- `update_pin_requests()`: Update pin requests for miners
- `storage_unpin_request()`: Remove file storage requests
- `add_to_blacklist()`: Blacklist specific file hashes

### Offchain Worker Features
- Periodic IPFS node synchronization
- Automatic file pinning and unpinning
- Garbage collection for IPFS repository
- Node identity and hardware info tracking

## Storage Components

### Key Storage Maps
- `FileSize`: Track file sizes
- `RequestedPin`: Store storage request details
- `FileStored`: Track files stored by miners
- `Blacklist`: Manage blacklisted file hashes
- `UnpinRequests`: Track unpin requests

## Events

Comprehensive event tracking for:
- Pin and unpin requests
- Miner file storage updates
- File blacklisting
- Node synchronization

## Error Handling

Robust error management covering scenarios like:
- Duplicate pin requests
- Node registration issues
- Insufficient replicas
- Blacklisted files

## Usage Example

```rust
// Request file storage
fn store_file(origin, file_hash, replicas) -> DispatchResult {
    let who = ensure_signed(origin)?;
    Pallet::<T>::add_ipfs_pin_request(
        who, 
        file_hash, 
        node_identity, 
        replicas
    )
}

// Blacklist a file
fn blacklist_file(origin, file_hash) -> DispatchResult {
    ensure_root(origin)?;
    Pallet::<T>::add_to_blacklist(file_hash)
}
