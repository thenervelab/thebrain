//! Per-family payout weight: the sum of the node weights of a family's
//! eligible children.
//!
//! This is the weight the bank (`pallet-hippocampus::pay_storage_miners`)
//! distributes storage emission by — a family's share of a payout is its
//! summed child weight over the summed child weight of every family, so the
//! family running the most/best-weighted nodes is paid the most.
//!
//! Deliberately NOT [`FamilyWeight`](crate::pallet::FamilyWeight): that is the
//! EMA-smoothed, delta-clamped *quality score* published to external consumers
//! (e.g. Bittensor). Emission is paid on the raw sum of what the family's nodes
//! are currently weighted at, so the payout tracks the validator-reported node
//! weights directly with no smoothing lag.
//!
//! The aggregation is a free function over iterators rather than a method on
//! `Pallet<T>` so it can be unit-tested without a mock runtime — Arion's
//! `Config` inherits `pallet_registration::Config`, whose own supertraits
//! (babe / staking / credits / balances / proxy) make a test runtime for this
//! pallet impractical.

use sp_std::prelude::*;

/// Sum each family's eligible child weights.
///
/// * `families` — every family and its child list.
/// * `is_eligible` — whether a child counts **toward this family**: called
///   with `(family, child)` so the implementation can confirm the child's own
///   registration names the family whose list it was found under, on top of
///   the Active/freshness checks.
/// * `node_weight` — the child's latest reported node weight.
///
/// Families whose eligible children sum to zero are **dropped**, not returned
/// with a zero weight: the bank caps how many payees one payout may carry
/// (`MaxMinersPerPayout`) and rejects the whole call above it, so padding the
/// list with families that would receive nothing can fail a payout that had
/// room for every family that actually earns.
///
/// Weights are widened to `u128` before summing. `u16` is the width of one
/// *node* weight, not of a family: at the runtime's bounds (35 children x
/// `MaxNodeWeight` 50_000) a family total reaches 1_750_000 and would saturate
/// a `u16` accumulator at 65_535, flattening every large family onto the same
/// value and silently equalizing shares the payout is supposed to separate.
pub fn sum_family_weights<AccountId, Children>(
	families: impl Iterator<Item = (AccountId, Children)>,
	is_eligible: impl Fn(&AccountId, &AccountId) -> bool,
	node_weight: impl Fn(&AccountId) -> u16,
) -> Vec<(AccountId, u128)>
where
	Children: IntoIterator<Item = AccountId>,
{
	let mut out = Vec::new();
	for (family, children) in families {
		let mut total: u128 = 0;
		for child in children {
			if !is_eligible(&family, &child) {
				continue;
			}
			total = total.saturating_add(u128::from(node_weight(&child)));
		}
		if total > 0 {
			out.push((family, total));
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use sp_std::collections::btree_map::BTreeMap;

	/// Children keyed by id: `(eligible, weight)`.
	fn env(entries: &[(u8, bool, u16)]) -> (BTreeMap<u8, bool>, BTreeMap<u8, u16>) {
		let mut eligible = BTreeMap::new();
		let mut weights = BTreeMap::new();
		for (id, ok, w) in entries {
			eligible.insert(*id, *ok);
			weights.insert(*id, *w);
		}
		(eligible, weights)
	}

	#[test]
	fn sums_every_eligible_child_of_a_family() {
		let (eligible, weights) = env(&[(1, true, 10), (2, true, 25), (3, true, 5)]);
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2, 3])].into_iter(),
			|_f, c| eligible[c],
			|c| weights[c],
		);
		assert_eq!(out, vec![(100, 40)]);
	}

	#[test]
	fn ineligible_children_contribute_nothing() {
		// Child 2 is deregistered/stale: its 25 must not be paid for.
		let (eligible, weights) = env(&[(1, true, 10), (2, false, 25), (3, true, 5)]);
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2, 3])].into_iter(),
			|_f, c| eligible[c],
			|c| weights[c],
		);
		assert_eq!(out, vec![(100, 15)]);
	}

	#[test]
	fn family_with_no_eligible_children_is_dropped() {
		let (eligible, weights) = env(&[(1, false, 10), (2, false, 25)]);
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2])].into_iter(),
			|_f, c| eligible[c],
			|c| weights[c],
		);
		assert!(out.is_empty());
	}

	#[test]
	fn family_whose_children_all_weigh_zero_is_dropped() {
		// Eligible but unweighted (never scored) — still nothing to pay.
		let (eligible, weights) = env(&[(1, true, 0), (2, true, 0)]);
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2])].into_iter(),
			|_f, c| eligible[c],
			|c| weights[c],
		);
		assert!(out.is_empty());
	}

	#[test]
	fn family_with_no_children_is_dropped() {
		let out =
			sum_family_weights(vec![(100u8, Vec::<u8>::new())].into_iter(), |_, _| true, |_| 10);
		assert!(out.is_empty());
	}

	#[test]
	fn each_family_is_summed_independently() {
		let (eligible, weights) =
			env(&[(1, true, 10), (2, true, 10), (3, true, 30), (4, false, 1_000)]);
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2]), (200u8, vec![3u8, 4])].into_iter(),
			|_f, c| eligible[c],
			|c| weights[c],
		);
		// Two nodes at 10 lose to one node at 30 — share follows summed
		// weight, not child count.
		assert_eq!(out, vec![(100, 20), (200, 30)]);
	}

	#[test]
	fn family_total_exceeds_u16_without_saturating() {
		// 35 children (the runtime's MaxChildrenPerFamily) at MaxNodeWeight.
		let children: Vec<u8> = (1..=35).collect();
		let out =
			sum_family_weights(vec![(100u8, children)].into_iter(), |_, _| true, |_| 50_000);
		assert_eq!(out, vec![(100, 1_750_000)]);
		// The point of the u128 accumulator: a u16 one would have stopped here.
		assert!(out[0].1 > u128::from(u16::MAX));
	}

	#[test]
	fn a_child_registered_to_another_family_is_not_counted() {
		// The child list is one index and the child's own registration is
		// another. If they ever disagree, the registration wins: a family must
		// never be credited for weight belonging to a child it does not own.
		let owner: BTreeMap<u8, u8> = [(1u8, 100u8), (2, 100), (3, 200)].into_iter().collect();
		let out = sum_family_weights(
			// Family 100's list wrongly contains child 3, which is registered
			// to family 200.
			vec![(100u8, vec![1u8, 2, 3])].into_iter(),
			|f, c| owner[c] == *f,
			|_| 10,
		);
		assert_eq!(out, vec![(100, 20)]);
	}

	#[test]
	fn larger_family_total_wins_a_bigger_share() {
		// Same per-node weight on both sides: three nodes beat one, 3:1.
		let out = sum_family_weights(
			vec![(100u8, vec![1u8, 2, 3]), (200u8, vec![4u8])].into_iter(),
			|_, _| true,
			|_| 50_000,
		);
		let small = out.iter().find(|(f, _)| *f == 200).unwrap().1;
		let large = out.iter().find(|(f, _)| *f == 100).unwrap().1;
		assert_eq!(large, small * 3);
	}
}
