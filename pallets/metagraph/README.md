# Metagraph Pallet

## Overview

The Metagraph Pallet is a sophisticated blockchain module designed to manage and synchronize network node information, roles, and consensus mechanisms. It provides a robust system for tracking, validating, and managing network participants across different roles.

## Key Features

- Dynamic UID (Unique Identifier) Management
- Consensus-Driven Node Validation
- Offchain Worker Integration
- Validator Trust and Reputation System
- Dividend and Node Role Tracking

## Extrinsics

### 1. `submit_hot_keys`
- Submits network UIDs from offchain sources
- Performs consensus validation
- Updates network node information
- Tracks validators and miners

### 2. `set_stored_dividends`
- Root-only function to set network dividends
- Stores dividend information for network participants

## Core Mechanisms

- Offchain Worker Synchronization
- Periodic UID and Dividend Fetching
- Consensus Validation Process
- Dynamic Node Role Assignment
- Validator Trust Point System

## Storage Components

- `UIDs`: Stores registered unique identifiers
- `ValidatorSubmissions`: Tracks validator node submissions
- `ValidatorTrustPoints`: Manages validator reputation
- `StoredDividends`: Stores network dividend information

## Unique Capabilities

- Automatic Node Role Detection
- Consensus-Based Network Validation
- Dynamic Validator Slashing Mechanism
- Flexible Node Information Synchronization

## Events

- `HotKeysUpdated`: Indicates network UID updates
- `StorageUpdated`: Signals storage changes
- `ValidatorTrustUpdated`: Tracks validator trust point modifications

## Configuration

- Configurable UID Submission Interval
- External Finney URL for Data Fetching
- Custom Storage Keys
