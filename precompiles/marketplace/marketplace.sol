// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;
address constant IMarketplace_ADDRESS = 0x0000000000000000000000000000000000000823;

IMarketplace constant IMarketplace_CONTRACT = IMarketplace(
    IMarketplace_ADDRESS
);

/// @title IMarketplace Interface Precompile
/// @dev This interface represents the precompiled contract that exposes IMarketplace functionality.
/// The precompile contract is expected to be deployed at a fixed address.
interface IMarketplace {
    // Get point balance for an address
    // function get_point_balance(address who) external view returns (uint128);
    function get_user_subscription(address user, uint32 subscriptionId) external view returns (
        uint32 id,
        uint8 tier,
        uint32 storageUsed,
        uint32 bandwidthUsed,
        uint32 requestsUsed,
        uint64 expiresAt,
        bool autoRenewal,
        bool active
    );

    function get_active_subscriptions(address user) external view returns (uint32[] memory);

    function get_package(uint8 _tier) external view returns (
        uint8 tier,
        uint32 storageGb,
        uint32 bandwidthGb,
        uint32 requestsLimit,
        uint32 pricePoints,
        uint32 durationMonths
    );

    function purchase_package(uint8 tier, uint32 cdnLocationId, bytes calldata referralCode) external payable;

    // function purchase_points(uint128 amount) external;
    function set_auto_renewal(uint32 subscriptionId, bool enabled) external payable;

    function storage_request(
        address owner,
        bytes[] calldata fileHashes,
        uint32 subscriptionId
    ) external;

  
    function storage_unpin_request(bytes calldata fileHash) external;

    function get_subscription_storage(address user, uint32 subscriptionId) external view returns (uint32 totalStorageGb, uint32 storageUsed);
    
    function get_next_renewal_block(address user, uint32 subscriptionId) external view returns (uint32);

    // Create a referral code for the caller
    function create_referral_code() external;
    
    // Change the referral code of the caller
    function change_referral_code() external;
    
    /// @dev Gets the total number of referral codes created
    /// @return The total number of referral codes
    function get_total_referral_codes() external view returns (uint32);
    
    /// @dev Gets the total rewards earned through referrals
    /// @return The total amount of rewards
    function get_total_referral_rewards() external view returns (uint128);
    
    /// @dev Gets the rewards earned by a specific referral code
    /// @param referralCode The referral code to check
    /// @return The amount of rewards earned by this code
    function get_referral_code_rewards(bytes memory referralCode) external view returns (uint128);
    
    /// @dev Gets the number of times a referral code has been used
    /// @param referralCode The referral code to check
    /// @return The usage count of the referral code
    function get_referral_code_usage_count(bytes memory referralCode) external view returns (uint32);

    /// @dev Gets the current balance of the marketplace pallet
    /// @return The balance of the marketplace pot in native currency
    function get_pot_balance() external view returns (uint128);

    /// @dev Transfer subscription ownership to another account
    /// @param to The address to transfer the subscription to
    /// @param subscriptionId The ID of the subscription to transfer
    function transfer_subscription(address to, uint32 subscriptionId) external;


    /// @dev Grant access to a subscription
    /// @param subscriptionId The ID of the subscription
    /// @param to The address to grant access to
    /// @param permission The permission level (0 = Read, 1 = Write, 2 = Admin)
    function grant_access(uint32 subscriptionId, address to, uint8 permission) external;

    /// @dev Revoke access from a subscription
    /// @param subscriptionId The ID of the subscription
    /// @param from The address to revoke access from
    function revoke_access(uint32 subscriptionId, address from) external;


    /// @dev Update access permission for an existing user
    /// @param subscriptionId The ID of the subscription
    /// @param user The address of the user whose permission is being updated
    /// @param newPermission The new permission level (0 = Read, 1 = Write, 2 = Admin)
    function update_access_permission(uint32 subscriptionId, address user, uint8 newPermission) external;


    /// @dev Switch to a different package tier
    /// @param subscriptionId The ID of the subscription to switch
    /// @param newTier The new package tier (0 = Starter, 1 = Quantum, 2 = Pinnacle)
    function switch_package(uint32 subscriptionId, uint8 newTier) external payable;

    /// @notice Upgrade package storage capacity
    /// @param gbsNeeded Additional gigabytes of storage needed
    function increase_package_storage(uint32 gbsNeeded) external;
}

struct StorageRequest {
    uint32 totalReplicas;
    uint32 fulfilledReplicas;
    address owner;
    bytes fileHash;
    bool isApproved;
    uint32 subscriptionId;
}
