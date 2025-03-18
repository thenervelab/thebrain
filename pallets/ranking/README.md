# Ranking Pallet

## Overview

The Ranking Pallet is a critical component of the blockchain infrastructure designed to manage and track node rankings within the network. It provides a robust mechanism for calculating, storing, and distributing rewards based on node performance and contributions.

## Key Features

- Dynamic Node Ranking System
- Weighted Ranking Calculation
- Reward Distribution Mechanism
- Support for Multiple Node Types (Relay and Miner Nodes)

## Extrinsics

### 1. `update_rankings`
- Updates the rankings of nodes in the network
- Requires:
  - Node IDs
  - Node Weights
  - Node Types
  - Node SS58 Addresses
- Sorts nodes based on their weights
- Assigns ranks to nodes

### 2. `update_rank_distribution_limit`
- Root-only function to update the maximum number of nodes eligible for reward distribution
- Allows dynamic adjustment of reward distribution scope

## Reward Distribution Mechanism

- Separate reward pools for Relay and Miner nodes
- Configurable reward percentages
- Rewards distributed proportionally based on node weights
- Rewards are added to node staking balance

## Storage Components

- `Rankings`: Stores individual node rankings
- `RankedList`: Maintains a sorted list of nodes by weight
- `LastGlobalUpdate`: Tracks the timestamp of the last ranking update
- `RewardsRecord`: Maintains historical reward information for nodes

## Configuration Parameters

- Relay Nodes Reward Percentage
- Miner Nodes Reward Percentage
- Rank Distribution Limit
- Pallet Instance ID

## Events

- `RankingsUpdated`: Emitted when node rankings are updated
- `RewardDistributed`: Emitted when rewards are distributed to nodes
- `RankDistributionLimitUpdated`: Emitted when the distribution limit is changed

## Dependencies

- `frame_system`
- `pallet_balances`
- `pallet_staking`
- `pallet_registration`
