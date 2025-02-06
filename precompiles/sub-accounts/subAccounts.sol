// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;
address constant ISubAccounts_ADDRESS = 0x0000000000000000000000000000000000000824;

ISubAccounts constant ISubAccounts_CONTRACT = ISubAccounts(
    ISubAccounts_ADDRESS
);

/// @title ISubAccounts Interface Precompile
/// @dev This interface represents the precompiled contract that exposes IMarketplace functionality.
/// The precompile contract is expected to be deployed at a fixed address.
interface ISubAccounts {
    /// @notice Add a new sub-account for a main account
    /// @param main The main account address that will own the sub-account
    /// @param newSubAccount The address to be added as a sub-account
    /// @dev The caller must be the main account or an existing sub-account of main
    /// @dev Reverts if newSubAccount is already a sub-account of any main account
    function add_sub_account(address main, address newSubAccount) external;

    /// @notice Remove an existing sub-account from a main account
    /// @param main The main account address that owns the sub-account
    /// @param subAccountToRemove The sub-account address to be removed
    /// @dev The caller must be the main account or an existing sub-account of main
    /// @dev Reverts if subAccountToRemove is not a sub-account of main
    function remove_sub_account(address main, address subAccountToRemove) external;

    /// @notice Check if an address is a sub-account of a main account
    /// @param sender The address to check
    /// @param main The main account address
    /// @return true if sender is a sub-account of main, false otherwise
    function isSubAccount(address sender, address main) external view returns (bool);

    /// @notice Check if an address is already registered as a sub-account
    /// @param who The address to check
    /// @return true if the address is already a sub-account, false otherwise
    function isAlreadySubAccount(address who) external view returns (bool);

    /// @notice Get the main account for a given sub-account
    /// @param who The sub-account address to check
    /// @return The main account address
    /// @dev Reverts if the address is not a sub-account
    function getMainAccount(address who) external view returns (address);
}