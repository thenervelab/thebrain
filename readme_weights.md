# Hippius Subnet Weight Calculation

## How It Works (Summary)

Think of the Hippius weight system like a restaurant rating that determines how much of the tip pool each waiter gets.

First, a **manager (validator)** watches each waiter and scores them on two things: how many customers they served (bandwidth, 70%) and how many tables they manage (storage, 30%). The scoring uses a "diminishing returns" curve -- going from 0 to 10 tables matters a lot more than going from 100 to 110.

Waiters can work in **teams (families)**. A team's reputation is built from its best members, but each additional member counts a little less than the previous one (80% decay). Only the top 10 members count. The team score is smoothed over time so one bad or good shift doesn't swing things wildly.

The **tip pool** itself isn't fixed -- it depends on how busy the restaurant is. When the restaurant is full (network stores lots of data), waiters get a bigger share. When it's slow, more goes to the house (validator).

Finally, each waiter's cut is simply their team's share of the total: `your_team_score / all_teams_scores * tip_pool`. This gets recalculated every 6 hours, and the rewards are distributed automatically.

> Code links reference commit [`dd67178`](https://github.com/thenervelab/thebrain/commit/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22). Line numbers may drift as the codebase evolves.

---

## Architecture Overview

```
Validator observes miner nodes
         |
         v
+-------------------------------------+
| Layer 1: Node Quality Scoring       |  pallets/arion-pallet
| Per-node weight from bandwidth,     |
| storage, uptime, and penalties      |
+-------------------------------------+
         |
         v
+-------------------------------------+
| Layer 2: Family Aggregation         |  pallets/arion-pallet
| Top-N nodes with rank decay         |
| -> EMA smoothing -> delta clamp     |
+-------------------------------------+
         |
         v
+-------------------------------------+
| Layer 3: Pool Economics             |  pallets/execution-unit
| Burn % splits 65,535 budget into    |
| miners_pool + uid_zero_pool         |
+-------------------------------------+
         |
         v
+-------------------------------------+
| Layer 4: Weight Orchestration       |  pallets/bittensor
| Per-type calculation, offline       |
| detection, grace period             |
+-------------------------------------+
         |
         v
+-------------------------------------+
| Layer 5: Rankings & Rewards         |  pallets/ranking
| Sort by weight, distribute rewards  |
| every 6 hours proportionally        |
+-------------------------------------+
         |
         v
+-------------------------------------+
| Bittensor Submission                |  offchain worker +
| Submit weights to subnet 75         |  vali-weights-submitter
+-------------------------------------+
```

**Families**: One SS58 account (the "family owner") can register multiple child miner nodes. Weights are computed per-node, then aggregated per-family. A miner's final weight depends on its family's collective performance.

---

## Layer 1: Node Quality Scoring (Arion Pallet)

[`compute_node_weight_from_quality()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1582)

Validators periodically report quality metrics for each miner node. These metrics are submitted on-chain via `submit_node_quality()` -- miners cannot self-report.

### Input: NodeQuality

[`NodeQuality` struct](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L440)

| Field | Type | Description |
|-------|------|-------------|
| `shard_data_bytes` | u128 | Total stored bytes on this node |
| `bandwidth_bytes` | u128 | Served bandwidth bytes in reporting window |
| `uptime_permille` | u16 | Availability (0--1000, where 1000 = 100%) |
| `strikes` | u32 | Validator-issued strike count |
| `integrity_fails` | u32 | Data integrity failure count |

### Scoring Formula

The formula uses **log2(1 + x)** to produce diminishing returns. This prevents a single node from dominating by simply adding more resources -- doubling your bandwidth gives roughly +1 unit of score, not double the score.

[`log2_fixed_u128()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1558)

```
Step 1: Concave scores (fixed-point 8.8 format, range 0..~127)
  bw_score  = log2(bandwidth_bytes + 1)
  st_score  = log2(shard_data_bytes + 1)

Step 2: Weighted blend (bandwidth 70%, storage 30%)
  weighted_sum = bw_score * 700 + st_score * 300
  denom        = 700 + 300 = 1000
  raw_score    = weighted_sum * NodeScoreScale / (denom * 127)

Step 3: Uptime multiplier
  score = raw_score * uptime_permille / 1000

Step 4: Penalties (subtracted directly)
  penalty     = strikes * StrikePenalty + integrity_fails * IntegrityFailPenalty
  node_weight = max(0, score - penalty)

Step 5: Cap
  node_weight = min(node_weight, MaxNodeWeight)
```

### Worked Example

A node with 10 GB bandwidth, 50 GB storage, 95% uptime, 0 strikes, 0 integrity fails:

```
bw_score  = log2(10,737,418,240 + 1)  ~= 33 * 256 = ~8,480 (fixed-point)
            but simplified to integer part: ~33
st_score  = log2(53,687,091,200 + 1)  ~= 35

weighted_sum = 33 * 700 + 35 * 300 = 23,100 + 10,500 = 33,600
raw_score    = 33,600 * 700 / (1000 * 127) = 23,520,000 / 127,000 ~= 185
after uptime = 185 * 950 / 1000 = 175
penalties    = 0
node_weight  = 175
```

### Configuration Parameters

[Runtime config](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/runtime/mainnet/src/lib.rs#L294)

| Parameter | Value | Meaning |
|-----------|-------|---------|
| `NodeBandwidthWeightPermille` | 700 | 70% of the blend comes from bandwidth |
| `NodeStorageWeightPermille` | 300 | 30% of the blend comes from storage |
| `NodeScoreScale` | 700 | Scales the blended score to u16 range |
| `MaxNodeWeight` | 50,000 | Per-node weight cap |
| `StrikePenalty` | 50 | Weight points deducted per strike |
| `IntegrityFailPenalty` | 100 | Weight points deducted per integrity failure |

---

## Layer 2: Family Weight Aggregation

### Top-N with Rank Decay

[`compute_family_weight_from_nodes()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1489)

A family's raw weight is computed by aggregating its children's node weights with diminishing returns per rank:

```
1. Collect all child node weights (non-zero only)
2. Sort descending
3. Take top FamilyTopN (10) nodes
4. Apply rank decay:
   family_raw = w[0] + w[1] * 0.8 + w[2] * 0.64 + w[3] * 0.512 + ...
```

Each subsequent node contributes `FamilyRankDecayPermille / 1000 = 0.8` times the previous node's contribution factor.

**Worked example**: A family with 3 nodes having weights [1000, 800, 600]:

```
family_raw = 1000 * 1.0 + 800 * 0.8 + 600 * 0.64
           = 1000 + 640 + 384
           = 2024
```

### Newcomer Grace Floor

[`apply_node_weights_and_recompute()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1618)

If a family was first seen within `NewcomerGraceBuckets` (24) buckets of the current bucket, and the raw weight is greater than 0, the weight is floored at `NewcomerFloorWeight` (10). This ensures new families have a minimum presence while ramping up.

### EMA Smoothing

[`ema_permille_u16()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1479)

After computing the raw weight, an Exponential Moving Average prevents sudden spikes:

```
smoothed_weight = prev_weight * 0.7 + raw_weight * 0.3
```

`FamilyWeightEmaAlphaPermille` = 300 means 30% of the new value is blended with 70% of the previous. This makes family weights converge gradually rather than jumping.

### Delta Clamping

After EMA, the weight change per bucket is clamped to +/- `MaxFamilyWeightDeltaPerBucket` (100):

```
lo = prev_weight - 100
hi = prev_weight + 100
final_weight = clamp(ema_result, lo, hi)
```

Even if a family's raw weight jumps from 0 to 50,000, the on-chain `FamilyWeight` can only increase by 100 per bucket.

### Configuration Parameters

| Parameter | Value | Meaning |
|-----------|-------|---------|
| `FamilyTopN` | 10 | Only top 10 nodes count per family |
| `FamilyRankDecayPermille` | 800 | Each rank contributes 80% of the previous |
| `MaxFamilyWeight` | 65,535 | Absolute cap on family weight |
| `FamilyWeightEmaAlphaPermille` | 300 | 30% new, 70% previous |
| `MaxFamilyWeightDeltaPerBucket` | 100 | Max change per update bucket |
| `NewcomerGraceBuckets` | 24 | Grace period length in buckets |
| `NewcomerFloorWeight` | 10 | Minimum weight during grace period |

---

## Layer 3: Pool Economics

[`pool_context()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L32)

The total weight budget is `MAX_SCORE = 65,535`. This budget is split between miners and the validator (UID 0) based on a dynamically computed **burn percentage**.

### Burn Percentage Formula

```
storage_gb      = total_network_storage / 1,073,741,824
cost            = storage_gb * price_per_gb              (in tokens)
emissions       = alpha_price * 50                       (EMISSION_PERIOD)
burn_number     = max(0, emissions - cost)
burn_percentage = burn_number / emissions                (defaults to 0.2 if emissions = 0)
```

### Pool Split

```
uid_zero_pool = 65,535 * burn_percentage      (goes to validator / UID 0)
miners_pool   = 65,535 - uid_zero_pool         (distributed among miners)
```

**What this means for miners**: When the network stores more data (higher cost relative to emissions), the burn percentage decreases, giving miners a larger share of the weight pool. When emissions far exceed storage cost, more weight goes to the validator.

### Constants

[`weight_calculation.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L24)

| Constant | Value | Meaning |
|----------|-------|---------|
| `MAX_SCORE` | 65,535 | Total weight budget (u16 max) |
| `EMISSION_PERIOD` | 50 | Emission calculation period |
| `BYTES_PER_GB` | 1,073,741,824 | Standard bytes per GB |
| `TOKEN_DECIMALS` | 18 | Token precision for price conversion |

---

## Layer 4: Weight Orchestration (Bittensor Pallet)

[`calculate_weights_for_nodes()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L631)

The bittensor pallet orchestrates weight calculation for all miner types by calling separate functions per type.

### Per-Miner Weight Formula

[`calculate_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L82) and [`calculate_final_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L174)

Each miner's weight is calculated as a proportion of its family's Arion weight relative to the entire network:

```
miner_weight = (family_arion_weight / total_arion_weight) * miners_pool
```

Where:
- `family_arion_weight` = the EMA-smoothed family weight from Layer 2
- `total_arion_weight` = sum of all active families' weights ([`get_total_family_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1531))
- `miners_pool` = from Layer 3's pool split

### Per-Type Functions

| Miner Type | Function | Offline Buffer | Grace Period |
|------------|----------|----------------|--------------|
| Storage | [`calculate_storage_miner_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L125) | 3,000 blocks (~5 hours) | None (disabled) |
| Storage S3 | [`calculate_storage_s3_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L275) | 300 blocks (~30 min) | 80% reduction for first 1,000 blocks |
| Validator | [`calculate_validator_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L364) | 300 blocks (~30 min) | 80% reduction for first 1,000 blocks |
| GPU | [`calculate_gpu_miner_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L453) | 300 blocks (~30 min) | 80% reduction for first 1,000 blocks |
| Compute | [`calculate_compute_miner_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L542) | 300 blocks (~30 min) | 80% reduction for first 1,000 blocks |

Block time is 6 seconds. `MIN_BLOCKS_REGISTERED = 1,000` (~100 minutes).

### Offline Detection

If a miner's last heartbeat block is older than the offline buffer, the weight is set to **0**. Storage miners have a longer grace period (3,000 blocks / ~5 hours) than other types (300 blocks / ~30 minutes).

### New Miner Grace Period

For S3, GPU, compute, and validator types: if registered for fewer than 1,000 blocks, the weight is reduced by 80% (multiplied by 0.2, minimum weight of 1). This grace period is currently disabled for storage miners.

### UID 0 (Validator) Weight

[`uid_zero_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L147)

The validator receives the `uid_zero_pool` weight from Layer 3. This is capped so the total of all weights (miners + validator) does not exceed 65,535:

```
final_uid_zero = min(uid_zero_weight, 65535 - sum_of_all_miner_weights)
```

[Capping logic](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L246)

---

## Layer 5: Rankings & Reward Distribution

### Ranking Update

[`do_update_rankings()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/ranking/src/lib.rs#L396)

Weights from Layer 4 are submitted to the ranking pallet, which:

1. Creates a `NodeRankings` entry for each node with its weight
2. Sorts all nodes by weight **descending**
3. Assigns sequential ranks (1st, 2nd, 3rd, ...)
4. Stores the sorted list in `RankedList` on-chain

### Reward Distribution

[`on_initialize()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/ranking/src/lib.rs#L719)

Every `BlocksPerEra` (3,600 blocks = 6 hours at 6-second block time), the ranking pallet distributes rewards proportionally:

```
reward = (node_weight / total_type_weight) * pallet_balance
```

Rewards are staked to the node owner's account via the staking pallet (`bond` or `bond_extra`).

### Ranking Instances

The ranking pallet uses separate instances per miner type. Each instance has its own reward pool:

| Instance | Miner Type | Status |
|----------|-----------|--------|
| 1 | Storage miners | **Active** |
| 3 | Validators | **Active** |
| 2 | Compute miners | Disabled (commented out) |
| 4 | GPU miners | Disabled (commented out) |
| 5 | Storage S3 miners | Disabled (commented out) |

[Rankings save logic](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L929)

---

## Bittensor Subnet Submission

Weights are submitted to Bittensor subnet 75 through two paths:

### On-Chain Offchain Worker

[`offchain_worker()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L96) and [`get_signed_weight_hex()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L877)

The validator node's offchain worker periodically:

1. Calculates all weights via `calculate_weights_for_nodes()`
2. Saves rankings to the ranking pallet (instances 1 and 3)
3. Constructs and signs a Bittensor `set_weights` extrinsic
4. Submits to the Finney chain (Bittensor mainnet)

### External Python Submitter

[`vali-weights-submitter/weight_submitter.py`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/vali-weights-submitter/weight_submitter.py)

An alternative/backup submission path that:

1. Fetches the Bittensor metagraph for subnet 75 (all active neurons and their UIDs)
2. Queries the Hippius chain's `RankedList` storage for current weights
3. Matches SS58 addresses between both chains
4. Normalizes weights to u16 range proportionally
5. Submits via `SubtensorModule.set_weights`

---

## Practical Guide: Maximizing Your Weight

Ordered by impact on your final weight:

### 1. Bandwidth (70% of node score)

This is the dominant factor in the scoring formula. Serve more bandwidth in each reporting window. The log2 curve means the first few GB of bandwidth matter far more than going from 100 TB to 200 TB -- focus on consistent, reliable bandwidth rather than raw maximums.

### 2. Storage (30% of node score)

Store more shard data. The same log2 diminishing returns apply -- initial storage capacity matters most.

### 3. Uptime (multiplier on your score)

Your score is multiplied by `uptime_permille / 1000`:
- 100% uptime (1000): score unchanged
- 95% uptime (950): score reduced by 5%
- 50% uptime (500): score cut in half

### 4. Avoid Strikes and Integrity Failures

Each strike costs 50 weight points. Each integrity failure costs 100 points. These are subtracted directly from your node score. Maintain data integrity and respond correctly to validator challenges.

### 5. Stay Online

If your last heartbeat exceeds the offline buffer, your weight drops to **0**:
- Storage miners: 3,000 blocks (~5 hours)
- All other types: 300 blocks (~30 minutes)

No weight = no rewards for the next era.

### 6. Family Strategy: Quality Over Quantity

Adding more nodes to your family helps, but with diminishing returns:
- Best node: 100% contribution
- 2nd best: 80%
- 3rd best: 64%
- 4th best: 51.2%
- 5th best: 41.0%
- ...and only the top 10 count at all

The optimal strategy is a few high-quality nodes rather than many low-quality ones.

### 7. Patience During Ramp-Up

- New families get a floor weight of 10 during the first 24 buckets (newcomer grace)
- EMA smoothing means weight converges slowly (30% new, 70% previous per bucket)
- Delta clamping limits increases to +100 per bucket
- Non-storage miners face an 80% weight reduction for the first ~100 minutes

---

## Glossary

| Term | Definition |
|------|------------|
| **Family** | An SS58 account that owns one or more registered miner child nodes |
| **Arion** | The pallet responsible for family-based reputation, CRUSH map management, and weight computation |
| **Bucket** | A time unit for validator-reported stats (periodic, not per-block) |
| **Permille** | Parts per thousand (1000 = 100%) |
| **EMA** | Exponential Moving Average -- smoothing technique that blends previous and current values |
| **CRUSH** | Controlled Replication Under Scalable Hashing -- the placement algorithm for data shards |
| **UID 0** | The validator account in the Bittensor subnet; receives the burn-percentage portion of weights |
| **MAX_SCORE** | 65,535 (u16 max) -- the total weight budget per cycle |
| **Era** | Reward distribution period (every 3,600 blocks = 6 hours) |

---

## Code Reference Index

| Component | File | Key Functions |
|-----------|------|---------------|
| Node quality scoring | [`pallets/arion-pallet/src/lib.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs) | [`compute_node_weight_from_quality()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1582), [`log2_fixed_u128()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1558) |
| Family aggregation | [`pallets/arion-pallet/src/lib.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs) | [`compute_family_weight_from_nodes()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1489), [`ema_permille_u16()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1479), [`apply_node_weights_and_recompute()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/arion-pallet/src/lib.rs#L1618) |
| Pool economics | [`pallets/execution-unit/src/weight_calculation.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs) | [`pool_context()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L32), [`calculate_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L82), [`uid_zero_weight()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/execution-unit/src/weight_calculation.rs#L147) |
| Weight orchestration | [`pallets/bittensor/src/lib.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs) | [`calculate_weights_for_nodes()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L631), [`calculate_storage_miner_weights()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/bittensor/src/lib.rs#L125) |
| Rankings & rewards | [`pallets/ranking/src/lib.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/ranking/src/lib.rs) | [`do_update_rankings()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/ranking/src/lib.rs#L396), [`on_initialize()`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/pallets/ranking/src/lib.rs#L719) |
| Runtime config | [`runtime/mainnet/src/lib.rs`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/runtime/mainnet/src/lib.rs#L294) | Parameter declarations (lines 294--320, 723) |
| Bittensor submission | [`vali-weights-submitter/weight_submitter.py`](https://github.com/thenervelab/thebrain/blob/dd671787e9db7ce62cebe3f1ceb8744a38ef7c22/vali-weights-submitter/weight_submitter.py) | `WeightSubmitter.run()` |
