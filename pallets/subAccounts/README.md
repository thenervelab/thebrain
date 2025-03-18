# SubAccounts Pallet

## Overview

The SubAccounts pallet allows users to manage sub-accounts associated with a main account on a blockchain. This functionality enables efficient organization and control over multiple accounts, facilitating seamless extrinsic submissions.

## Goals

The primary goals of the SubAccounts pallet are:

- **Account Management**: Enable main accounts to add and remove sub-accounts.
- **User Control**: Provide a secure interface for users to manage their sub-accounts.
- **Event Tracking**: Emit events for actions related to sub-account management.

## Extrinsics

The SubAccounts pallet provides the following extrinsics:

### 1. `add_sub_account`

- **Description**: Allows the sender to add a new sub-account under their main account.
- **Parameters**:
  - `main`: The address of the main account.
  - `new_sub_account`: The address of the sub-account to be added.
- **Emits**: `SubAccountAdded` event when successful.

### 2. `remove_sub_account`

- **Description**: Allows the sender to remove an existing sub-account from their main account.
- **Parameters**:
  - `main`: The address of the main account.
  - `sub_account_to_remove`: The address of the sub-account to be removed.
- **Emits**: `SubAccountRemoved` event when successful.

## Error Handling

The pallet includes several error types for managing common issues:

- **NoSubAccount**: Raised when an operation requires a sub-account but none exists.
- **NotAllowed**: Raised when an unauthorized account attempts an action.
- **NoAccountsLeft**: Raised when trying to remove all sub-accounts.
- **AlreadySubAccount**: Raised when attempting to add an existing sub-account.

## Conclusion

The SubAccounts pallet is essential for managing user accounts within blockchain applications. By enabling users to efficiently add and remove sub-accounts, it enhances user control and simplifies account management. For further details, please refer to the documentation within this repository.
