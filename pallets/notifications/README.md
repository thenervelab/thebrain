# Notifications Pallet

## Overview

The Notifications Pallet is a blockchain module designed to manage and handle user notifications within the network. It provides a flexible system for sending, tracking, and managing notifications across different accounts.

## Key Features

- Send Notifications to Specific Accounts
- Mark Notifications as Read
- Sudo-level Notification Management
- Account Banning Mechanism
- Cooldown Period for Notification Sending
- Support for Recurring and One-time Notifications

## Extrinsics

### 1. `send_notification`
- Allows users to send notifications to other accounts
- Features:
  - Specify recipient
  - Set block to send notification
  - Optional recurrence settings
  - Cooldown period between notifications
- Prevents banned accounts from sending notifications

### 2. `mark_as_read`
- Allows users to mark their own notifications as read
- Requires notification index

### 3. `sudo_update_notification`
- Root-only function to update existing notifications
- Can modify notification parameters like block, recurrence, and frequency

### 4. `ban_account`
- Root-only function to ban an account from sending notifications

## Notification Types

- General Notifications
- Subscription Ending Notifications
- Subscription Ended Notifications

## Storage Components

- `Notifications`: Stores user notifications
- `BannedAccounts`: Tracks accounts banned from sending notifications
- `LastCallTime`: Tracks last notification send time for cooldown mechanism

## Events

- `NotificationSent`
- `NotificationRead`
- `SubscriptionHasEnded`
- `SubscriptionEndingSoon`
- `AccountBanned`

## Configuration

- Configurable Cooldown Period
