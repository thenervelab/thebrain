//! Compute miner settlement: types, traits and the pure encoding helpers.
//!
//! The SCALE layouts and domain tags follow the interface contract
//! (`hippius-backend/docs/design/compute-miner-pay-interfaces.md`, §3). The
//! backend signs [`approval_digest`] with an sr25519 approver key, vali submits
//! the approval and then the parts; the chain recomputes every digest itself.
//!
//! Domain tags are fixed bytes with **no** length prefix, followed by the
//! SCALE encoding of the fields.

use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{pallet_prelude::ConstU32, BoundedVec};
use scale_info::TypeInfo;
use sp_core::{H256, U256};
use sp_runtime::RuntimeDebug;
use sp_std::vec::Vec;

/// Domain tag of the signed approval payload.
pub const SETTLE_DOMAIN: &[u8] = b"hippius:compute-settle:v1";
/// Domain tag of a part hash.
pub const PART_DOMAIN: &[u8] = b"hippius:compute-part:v1";
/// Domain tag of the signed revocation payload.
pub const REVOKE_DOMAIN: &[u8] = b"hippius:compute-revoke:v1";
/// Domain tag of the params hash.
pub const PARAMS_DOMAIN: &[u8] = b"hippius:compute-params:v1";

/// The only formula version this runtime accepts.
pub const FORMULA_VERSION: u16 = 1;
/// Most parts one approval may have.
pub const MAX_PARTS: u32 = 16;
/// Most submitter accounts and most approver keys.
pub const MAX_SETTLEMENT_KEYS: u32 = 4;
/// Basis points in one whole.
pub const BPS: u32 = 10_000;
/// 18-decimal fixed point one.
pub const ONE: u128 = 1_000_000_000_000_000_000;

pub type PartHashes = BoundedVec<H256, ConstU32<MAX_PARTS>>;

/// What the backend signs for one `(window, revision)` (contract §3.2,
/// `ApprovalInputV1`). Field order is the SCALE order.
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct ApprovalInput {
	/// Number of parts; 0 means nothing to pay and completes at once.
	pub parts: u8,
	/// `part_hashes[i]` = [`part_hash`] of part `i`; length must equal `parts`.
	pub part_hashes: PartHashes,
	/// Σ entry usd over every part, in credits (1e18 = $1).
	pub usd: u128,
	/// The `AlphaPrice` the backend read; must equal the chain's fresh price.
	pub price: u128,
	pub payable_root: H256,
	pub evidence_root: H256,
	/// Last hourly document included in `payable_root`. Signed and recorded,
	/// not checked against chain state.
	pub upto_hour: u64,
	pub params_version: u32,
	/// Must be [`FORMULA_VERSION`].
	pub formula_version: u16,
	/// [`params_hash`] of the params of `params_version`.
	pub params_hash: H256,
}

/// One payee line of a part (contract §3.1, `EntryV1`).
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Entry<AccountId> {
	pub family: AccountId,
	/// Credits (1e18 = $1), strictly positive.
	pub usd: u128,
	/// `floor(usd × 1e18 / price)` with the approval's price.
	pub alpha: u128,
}

/// Payout parameters (contract §3.3 `ParamsV1`): exactly the fields the
/// params hash covers. The daily alpha cap is a separate root parameter.
#[derive(Clone, Copy, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct PayoutParams {
	/// Miner share `r`, basis points.
	pub share_bps: u16,
	/// Rounding tolerance `ε`, basis points.
	pub tol_bps: u16,
	/// List price of one vCPU-hour in µUSD (the RU_B unit).
	pub vcpu_hour_uusd: u64,
}

/// A stored approval (contract §4): the signed input, whose `price` is the
/// snapshot every part converts with, and the digest a revocation commits to.
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Approval {
	pub input: ApprovalInput,
	pub approval_digest: H256,
}

impl Approval {
	/// `floor(usd × 1e18 / price)`: the alpha the approval reserves.
	pub fn reserved_alpha(&self) -> u128 {
		alpha_for(self.input.usd, self.input.price).unwrap_or(0)
	}
}

/// Payment progress of one approval (contract §4 fields, then the chain's
/// completion day).
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Progress<AccountId> {
	/// Next part to pay.
	pub next_part: u8,
	/// Last family paid, for the strict ascending order across parts.
	/// Meaningless (all-zero) while `next_part == 0`.
	pub last_family: AccountId,
	/// Credits paid so far.
	pub usd_paid: u128,
	/// Alpha paid so far (transfers and owed alike).
	pub alpha_paid: u128,
	/// UTC day (window number) the approval completed, `None` while open.
	pub completed_day: Option<u32>,
}

/// A root-only settlement change, applied after `RootChangeDelay`.
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub enum RootChange<AccountId> {
	/// Replace the submitter accounts (vali) and the approver keys (backend).
	Keys {
		submitters: BoundedVec<AccountId, ConstU32<MAX_SETTLEMENT_KEYS>>,
		approvers: BoundedVec<sp_core::sr25519::Public, ConstU32<MAX_SETTLEMENT_KEYS>>,
	},
	/// Publish the next params version.
	Params(PayoutParams),
	/// Set the first window, once.
	FirstWindow(u32),
	/// Switch settlement on. Switching off is immediate (admin or root).
	Enable,
	/// Account `Defund` pays to.
	RevenueAccount(AccountId),
	/// Move unreserved funds from the settlement account to the revenue
	/// account, capped at `free − seed − approved unpaid`.
	Defund(u128),
	/// Most alpha settlement may pay per UTC day.
	MaxDailyAlpha(u128),
}

/// The slot a [`RootChange`] occupies in `PendingRootChange`.
#[derive(Clone, Copy, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub enum ChangeKind {
	Keys,
	Params,
	FirstWindow,
	Enable,
	RevenueAccount,
	Defund,
	MaxDailyAlpha,
}

impl<AccountId> RootChange<AccountId> {
	pub fn kind(&self) -> ChangeKind {
		match self {
			RootChange::Keys { .. } => ChangeKind::Keys,
			RootChange::Params(_) => ChangeKind::Params,
			RootChange::FirstWindow(_) => ChangeKind::FirstWindow,
			RootChange::Enable => ChangeKind::Enable,
			RootChange::RevenueAccount(_) => ChangeKind::RevenueAccount,
			RootChange::Defund(_) => ChangeKind::Defund,
			RootChange::MaxDailyAlpha(_) => ChangeKind::MaxDailyAlpha,
		}
	}
}

/// Value-backed compute revenue per source window (the marketplace).
pub trait ComputeRevenueSource {
	/// The UTC day the chain clock is in (`current_compute_period / 24`).
	fn current_window() -> u32;
	/// `charged(window)` once final (past `MaxComputeBillingLag`), else
	/// `None`.
	fn final_charged(window: u32) -> Option<u128>;
	/// Make `window` final with `charged` revenue (benchmarks only).
	#[cfg(feature = "runtime-benchmarks")]
	fn bench_set_final_charged(window: u32, charged: u128);
}

/// The chain's alpha price (`Credits.AlphaPrice`, credits per alpha).
pub trait AlphaPriceSource<BlockNumber> {
	/// The price and the block it was last written at (`None` = unknown,
	/// which settlement treats as stale).
	fn alpha_price() -> (u128, Option<BlockNumber>);
	/// Set a fresh price (benchmarks only).
	#[cfg(feature = "runtime-benchmarks")]
	fn bench_set_price(price: u128);
}

/// Other privileged roles a settlement key must not hold.
pub trait SettlementKeyRoles<AccountId> {
	/// True if `who` is a credits authority, a compute billing authority or a
	/// charge-cap admin.
	fn has_other_role(who: &AccountId) -> bool;
}

/// `floor(usd × 1e18 / price)` in U256; `None` for a zero price or overflow.
pub fn alpha_for(usd: u128, price: u128) -> Option<u128> {
	if price == 0 {
		return None;
	}
	let alpha = U256::from(usd).checked_mul(U256::from(ONE))? / U256::from(price);
	u128::try_from(alpha).ok()
}

/// The revenue cap, overflow-safe in U256:
/// `(paid + usd) × 10_000 ≤ charged × (share_bps + tol_bps)`.
pub fn within_revenue_cap(paid: u128, usd: u128, charged: u128, share_bps: u16, tol_bps: u16) -> bool {
	let lhs = (U256::from(paid) + U256::from(usd)) * U256::from(BPS);
	let rhs = U256::from(charged) * (U256::from(share_bps) + U256::from(tol_bps));
	lhs <= rhs
}

fn tagged(domain: &[u8], body: impl Encode) -> Vec<u8> {
	let mut msg = Vec::with_capacity(domain.len() + body.size_hint());
	msg.extend_from_slice(domain);
	body.encode_to(&mut msg);
	msg
}

/// `PARAMS_DOMAIN ‖ SCALE(share_bps, tol_bps, vcpu_hour_uusd)`.
pub fn params_message(p: &PayoutParams) -> Vec<u8> {
	tagged(PARAMS_DOMAIN, p)
}

pub fn params_hash(p: &PayoutParams) -> H256 {
	H256(sp_io::hashing::blake2_256(&params_message(p)))
}

/// `SETTLE_DOMAIN ‖ genesis ‖ window ‖ revision ‖ SCALE(input)`.
pub fn approval_message(genesis: &H256, window: u32, revision: u8, input: &ApprovalInput) -> Vec<u8> {
	tagged(SETTLE_DOMAIN, (genesis, window, revision, input))
}

pub fn approval_digest(genesis: &H256, window: u32, revision: u8, input: &ApprovalInput) -> H256 {
	H256(sp_io::hashing::blake2_256(&approval_message(genesis, window, revision, input)))
}

/// `PART_DOMAIN ‖ genesis ‖ window ‖ revision ‖ part ‖ SCALE(entries)`.
pub fn part_message<AccountId: Encode>(
	genesis: &H256,
	window: u32,
	revision: u8,
	part: u8,
	entries: &[Entry<AccountId>],
) -> Vec<u8> {
	tagged(PART_DOMAIN, (genesis, window, revision, part, entries))
}

pub fn part_hash<AccountId: Encode>(
	genesis: &H256,
	window: u32,
	revision: u8,
	part: u8,
	entries: &[Entry<AccountId>],
) -> H256 {
	H256(sp_io::hashing::blake2_256(&part_message(genesis, window, revision, part, entries)))
}

/// `REVOKE_DOMAIN ‖ genesis ‖ window ‖ revision ‖ approval_digest`.
pub fn revoke_digest(genesis: &H256, window: u32, revision: u8, approval_digest: &H256) -> H256 {
	H256(sp_io::hashing::blake2_256(&tagged(
		REVOKE_DOMAIN,
		(genesis, window, revision, approval_digest),
	)))
}
