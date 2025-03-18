# Compute Pallet

## Overview

The Compute Pallet is a critical component of the TheBrain blockchain infrastructure, designed to manage and orchestrate computational resources across a decentralized network. It provides a robust framework for creating, tracking, and managing compute requests, VM lifecycle management, and resource allocation.

## Features

- 🖥️ **Compute Request Management**
  - Create and track compute requests
  - Assign requests to miners
  - Monitor request status (Pending, InProgress, Running, Failed, Cancelled)

- 🌐 **VM Lifecycle Operations**
  - Create virtual machines
  - Start, stop, and delete VMs
  - Manage VM resources (CPU, RAM, Storage, Bandwidth)

- 🔒 **Secure Resource Allocation**
  - IP address management
  - Node registration integration
  - Offchain worker support for asynchronous operations

## Key Components

### Storage

- `ComputeRequests`: Tracks compute requests per user
- `MinerComputeRequests`: Manages compute requests for miners
- `MinerComputeDeletionRequests`: Handles VM deletion requests
- `MinerComputeStopRequests`: Manages VM stop requests
- `AvailableIps`: Maintains a pool of available IP addresses

### Offchain Workers

Supports offchain worker operations for:
- VM creation
- VM lifecycle management
- Asynchronous request processing