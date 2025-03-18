// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;
address constant IAccountProfile_ADDRESS = 0x0000000000000000000000000000000000000826;

IAccountProfile constant IAccountProfile_CONTRACT = IAccountProfile(
    IAccountProfile_ADDRESS
);

/// @title IAccountProfile Interface Precompile
/// @dev This interface represents the precompiled contract that exposes IMarketplace functionality.
/// The precompile contract is expected to be deployed at a fixed address.
interface IAccountProfile {
    /// @notice Alternative name for addPublicItem
    /// @param item The bytes data to store
    function add_public_item(bytes memory item) external;

    /// @notice Alternative name for addPrivateItem
    /// @param item The bytes data to store
    function add_private_item(bytes memory item) external;

    /// @notice Get the public item for an address
    /// @param account The address to get the public item for
    /// @return The stored public bytes data
    function get_public_item(address account) external view returns (bytes memory);

    /// @notice Get the private item for an address
    /// @param account The address to get the private item for
    /// @return The stored private bytes data
    function get_private_item(address account) external view returns (bytes memory);

    function setUsername(bytes memory username) external;

    function getStorage(address user) external view returns (bytes memory publicItem, bytes memory privateItem);
}