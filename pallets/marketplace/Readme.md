# Marketplace Pallet

## Overview

The Marketplace Pallet is a Substrate module that manages subscription plans,
credit-based billing, and storage accounting for a decentralised storage and
compute platform.

Plans come in three kinds, and an account may hold one of each at a time:

- **Drive** storage
- **S3** storage — a separate subscription slot from Drive, billed and changed
  independently
- **Compute** — occupies no storage slot

## Billing

### Date-to-date cycles

Subscriptions bill on their own anniversary rather than on the 1st of the
month. A plan bought on the 14th runs to the 14th of the next month and is
charged in full; there is no prorated first month.

Cycle length comes from the calendar at charge time, so a cycle is 28, 29, 30
or 31 days depending on the real month. An anchor day that does not exist in
the target month is clamped to that month's last day, and the anchor is
*remembered* rather than re-derived — so a subscriber anchored to the 31st
returns to the 31st after February rather than sticking on the 28th.

Subscriptions created before this change are anchored to the 1st and keep
billing on the 1st, unchanged. Both behaviours coexist.

### The due-day index

`DueAccounts` maps a due day to the accounts due that day, so the renewal sweep
reads only who is actually due instead of walking every subscription. The drain
is capped at `MaxSubscriptionChargesPerRun` accounts per tick and resumes from
`DueDayCursor`, which advances only over an empty day and never past today.

An index entry is a hint; the subscription is the truth. Charging re-reads the
subscription and re-checks that it is active and due, so a stale entry costs one
read rather than a wrong charge.

`commit_subscriptions` is the only write path for `UserAllSubscriptionPlans`,
and reconciles the index from the before/after diff — the index cannot drift by
someone forgetting to update it at a call site.

### Hourly pay-as-you-go

Users whose bytes are not covered by a plan are billed per GiB every hour.
Drive and S3 bytes are metered separately and each is exempted by its own plan,
so holding one plan still leaves the other side billable.

## Supported Extrinsics

### Plans and subscriptions
- `add_new_plan` — create a plan (root)
- `purchase_plan` — buy one or more plans (whitelisted caller)
- `change_storage_plan` — move a Drive subscription to another Drive plan
- `change_s3_plan` — move an S3 subscription to another S3 plan
- `cancel_user_subscription` — cancel, refunding unused prepaid cycles
- `set_package_suspension` — suspend or unsuspend a plan (root)

### Usage reporting
- `update_user_file_usage` — report one user's Drive and S3 usage
- `update_users_file_usage` — the batched form

### Credits
- `deposit` — credit an account, optionally with a referral code
- `chargeback` — reverse a deposit
- `create_referral_codes_for` — mint referral codes in bulk
- `retry_pending_sudo_refunds` — retry refunds the bank could not deliver

### Pricing (root)
- `set_price_per_gb` — hourly per-GiB storage price
- `set_bandwidth_price` — bandwidth price
- `set_storage_price_per_miner` — per-miner storage price
- `set_specific_miner_request_fee` — fee for a specific-miner request

### Administration (root)
- `set_sudo_key`
- `set_os_disk_image_url`
- `sudo_set_purchase_plan_enabled` — plan-purchase kill switch
- `sudo_set_storage_operations` — storage-operations kill switch
- `sudo_set_whitelist_canceller` / `sudo_remove_whitelist_canceller`
- `sudo_set_subscription_canceller`
- `sudo_set_referral_commission_rate`
- `sudo_set_referral_bank_floor`
