# Backup Pallet

## Overview

The Backup Pallet is a critical component of the TheBrain blockchain infrastructure, designed to provide robust, decentralized snapshot and backup management for computational resources. It enables secure, periodic snapshots of virtual machines and maintains comprehensive metadata about these backups.

## Features

- **Snapshot Management**
  - Create and track VM snapshots
  - Store snapshot metadata on-chain
  - Support for IPFS-based snapshot storage

- **Secure Backup Tracking**
  - Per-node and per-user backup metadata
  - Access control for backup ownership
  - Immutable snapshot records

- **Periodic Backup Operations**
  - Configurable offchain worker frequency
  - Automatic snapshot creation for eligible compute requests
  - Background backup processing

## Key Components

### Storage Structures

- `Backups`: Double-mapped storage for backup metadata
  - Tracks snapshots per node and user
  - Stores comprehensive snapshot information

- `BackupMetadata`: Detailed backup information
  - Owner identification
  - Snapshot list
  - Creation and update timestamps

- `SnapshotMetadata`: Individual snapshot details
  - IPFS Content Identifier (CID)
  - Block number
  - Optional description
  - Associated compute request ID

## Offchain Worker Functionality

- Periodic snapshot creation
- Background backup processing
- Automatic snapshot metadata management

## Usage Examples

### Adding a Snapshot

```rust
// Add a snapshot to a backup
pallet_backup::Pallet::<T>::add_snapshot(
    node_id,
    snapshot_cid,
    description,
    request_id,
    account_id
)
```

### Configuring Backup Frequency

```rust
// Set offchain worker backup frequency (in blocks)
pallet_backup::Pallet::<T>::set_backup_offchain_worker_frequency(new_frequency)
```

## Security Considerations

- Unsigned transaction validation
- Owner-based access control
- Immutable snapshot records
- Integration with IPFS for decentralized storage

## Performance

- Minimal on-chain storage footprint
- Efficient metadata management
- Configurable backup frequency

## Dependencies

- `pallet-marketplace`
- `pallet-compute`
- `frame-support`
- `frame-system`

## Compatibility

- Substrate Framework
- IPFS Integration
- Polkadot Ecosystem

## Roadmap

- [ ] Enhanced snapshot compression
- [ ] Multi-node backup synchronization
- [ ] Advanced backup restoration mechanisms

## License

MIT-0 License

## Contributing

Contributions are welcome! Please read our contributing guidelines before submitting pull requests.

## Release

polkadot v1.15.0
