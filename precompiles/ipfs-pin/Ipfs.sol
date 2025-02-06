// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

address constant IPFS_PIN_ADDRESS = 0x0000000000000000000000000000000000000807;

IPFS_PIN constant IPFS_PIN_CONTRACT = IPFS_PIN(
    IPFS_PIN_ADDRESS
);

/// @title IPFS_PIN Interface Precompile
/// @dev This interface represents the precompiled contract that exposes IPFS_PIN functionality.
/// The precompile contract is expected to be deployed at a fixed address.
interface IPFS_PIN {
    function call_storage_request(uint32 totalReplicas,uint32 fullfilledReplicas,bytes memory fileHash, address who) external;

    function call_ipfs_pin_request(bytes memory fileHash, bytes memory nodeIdentity) external;

    function read_files_pinned(bytes memory nodeIdentity) external view returns (bytes[] memory)  
}
