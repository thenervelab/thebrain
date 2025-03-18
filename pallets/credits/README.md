# Credits Pallet

## Overview

The Credits Pallet is a flexible credit management system designed to handle user credits within our ecosystem. It provides mechanisms for minting and burning credits through a decentralized authority system.

## Key Features

- **Credit Management**: 
  - Mint new credits
  - Burn existing credits
- **Authority System**: 
  - Multiple authorized accounts can mint and burn credits
  - Sudo-controlled authority management
- **Flexible Credit Tracking**: 
  - Single storage for free credits
  - Secure access controls
- **Safety Mechanisms**:
  - Prevents burning credits beyond available balance
  - Ensures credit integrity

## Extrinsics

1. **add_authority(authority)**
   - Allows sudo to add a new authority account
   - Prevents duplicate authorities
   - Only callable by sudo account

2. **remove_authority(authority)**
   - Allows sudo to remove an existing authority account
   - Prevents removing non-existent authorities
   - Only callable by sudo account

3. **mint(who, amount)**
   - Allows authorized accounts to create credits for a specific account
   - Increases free credits balance
   - Only callable by registered authority accounts

4. **burn(who, amount)**
   - Allows authorized accounts to destroy credits from a specific account
   - Reduces free credits balance
   - Prevents burning more credits than available
   - Only callable by registered authority accounts

5. **convert_balance_to_credits(amount)**
   - Allows users to convert native tokens to credits
   - Burns the specified amount of native tokens
   - Mints an equivalent amount of credits to the user's account
   - Requires sufficient native token balance
   - Prevents conversion of zero or negative amounts

## Error Handling

- `InsufficientFreeCredits`: Prevents operations exceeding free credit balance
- `NotAuthorized`: Prevents unauthorized credit operations
- `AuthorityAlreadyExists`: Prevents adding duplicate authorities
- `AuthorityNotFound`: Handles attempts to remove non-existent authorities
- `InvalidConversionAmount`: Prevents converting zero or negative amounts
- `InsufficientBalance`: Ensures user has enough native tokens for conversion

## Design Principles

- **Security**: Strict access controls with granular authority management
- **Flexibility**: Decentralized credit management through multiple authorities
- **Simplicity**: Straightforward credit tracking mechanism

## Helper Functions

- `get_free_credits(account)`: Retrieve the free credits for a specific account
- `increase_user_credits(account, amount)`: Increase credits for an account
- `decrease_user_credits(account, amount)`: Decrease credits for an account
- `ensure_is_authority(authority)`: Validate if an account is an authorized authority

## Events

- `MinetdAccountCredits`: Emitted when credits are minted to an account
- `BurnedAccountCredits`: Emitted when credits are burned from an account
- `AuthorityAdded`: Emitted when a new authority is added
- `AuthorityRemoved`: Emitted when an authority is removed