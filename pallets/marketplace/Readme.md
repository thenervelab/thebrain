# Marketplace Pallet

## Overview
he Marketplace Pallet is a Substrate blockchain module designed to manage subscriptions, storage requests, and access control for a decentralized storage and compute platform. It provides a comprehensive solution for users to purchase, manage, and share subscription plans with granular access permissions.

This pallet offers two types of plans: Storage and Compute, allowing users to tailor their subscriptions to their specific needs.

## Key Features

### Subscription Management
- Create, transfer, and cancel subscriptions
- Set auto-renewal status
- Manage subscription access permissions

### Storage Requests
- Request file storage with specified replicas
- Unpin and manage stored files
- Track storage usage within subscription limits

### Referral System
- Generate unique referral codes
- Track referral code usage and rewards

### Plan Management
- Add and update storage and compute plans
- Purchase plans using credits
- Modify plan pricing

## Supported Extrinsics
- `set_auto_renewal`: Update subscription auto-renewal status
- `create_referral_code`: Generate a unique referral code
- `change_referral_code`: Modify an existing referral code
- `transfer_subscription`: Transfer subscription ownership
- `grant_access`: Grant access to a subscription
- `revoke_access`: Remove access from a subscription
- `update_access_permission`: Modify user access level
- `storage_request`: Request file storage
- `storage_unpin_request`: Remove stored files
- `update_storage_plan_price_per_gb`: Modify storage plan pricing
- `update_compute_plan_price_per_block`: Modify compute plan pricing
- `add_new_plan`: Create a new subscription plan
- `purchase_plan`: Buy a subscription plan

