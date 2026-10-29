#![cfg_attr(not(feature = "std"), no_std)]

//! # Hippocampus Pallet (the bank)
//!
//! Holds the funds used to pay Arion storage miners and marketplace
//! referral commissions.
//!
//! - `deposit(amount, deposit_type)`: anyone can fund the bank sovereign account,
//!   tagging the deposit with its source (`DepositType`).
//! - `request_payment(requester, dest, amount)`: internal API (not an extrinsic) —
//!   only whitelisted requester accounts (e.g. the Arion pallet sovereign account)
//!   can pull funds. Pays out at most the available balance and returns the amount
//!   actually paid; the caller is responsible for handling shortfalls. Fails with
//!   `DistributionDisabled` while the global switch is off.
//! - `pay_storage_miners(amount)`: admin-gated; distributes `amount` from the
//!   emission compartment to Arion storage families pro-rata by weight — a
//!   family's weight is the sum of the node weights of its eligible children,
//!   so the operator running the most/best-weighted nodes is paid the most.
//! - `pay_compute_miners(amount)`: admin-gated; distributes `amount` from the
//!   *compute* emission compartment to compute miners pro-rata by their
//!   epoch reward weight. Its compartment (`DepositType::ComputeEmission`)
//!   and its ledger (`ComputeEmissionPaidOut`) are separate from the storage
//!   ones — neither payout can spend the other's funds — but the two share a
//!   single 24-hour rate limit (`Max24HourMinerPayout`), so total emission
//!   leaving the bank per day is capped once across both.
//! - `add_requester` / `remove_requester`: admin-gated whitelist management.
//! - `set_distribution_enabled`: admin-gated global switch. While off, every
//!   `request_payment` is rejected and no funds leave the bank; deposits are
//!   unaffected.
//!
//! ## Compute settlement (calls 10-18)
//!
//! Pays compute miners from compute revenue, per source window (UTC day),
//! from its **own** sub-accounts (`cmp-settle`, `cmp-owed`), never from the
//! bank account above, so neither `pay_compute_miners` nor any other bank path
//! can reach these funds, and settlement cannot reach the bank's.
//!
//! - The backend signs an approval per `(window, revision)` with an sr25519
//!   approver key; a submitter (vali) submits it. The chain checks both
//!   switches (`ComputeSettlementEnabled` and `DistributionEnabled`), key
//!   roles, order, params, revenue finality, the revenue cap
//!   `paid + usd ≤ (r + tol) × charged(W)` and the alpha price.
//! - Parts are paid in order; each must hash to the approved part hash, list
//!   families strictly ascending, convert at the approval's price, stay
//!   within the approval's total (equal on the last part), the settlement
//!   balance and the daily alpha cap. A failed transfer is moved to the owed
//!   account and claimable once.
//! - Keys, params, the first window, enabling and defunding are root-only and
//!   take effect `RootChangeDelay` after scheduling; the admin may only
//!   disable.

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod settlement_tests;

#[cfg(test)]
mod settlement_vectors;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod settlement;
pub mod weights;

pub use pallet::*;
pub use settlement::{
	AlphaPriceSource, Approval, ApprovalInput, ChangeKind, ComputeRevenueSource, Entry,
	PayoutParams, Progress, RootChange, SettlementKeyRoles,
};
pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {
	use crate::settlement::{
		self as st, AlphaPriceSource, ComputeRevenueSource, SettlementKeyRoles, MAX_SETTLEMENT_KEYS,
	};
	use crate::weights::{SettlementWeightInfo, WeightInfo};
	use frame_support::{
		dispatch::{DispatchResultWithPostInfo, Pays},
		pallet_prelude::*,
		storage::with_storage_layer,
		traits::{Currency, ExistenceRequirement, Get},
		PalletId,
	};
	use frame_system::pallet_prelude::*;
	use payment_math::Tokens;
	use sp_core::{sr25519, H256};
	use sp_runtime::traits::{AccountIdConversion, SaturatedConversion, Saturating, Zero};
	use sp_std::vec::Vec;

	pub type BalanceOf<T> =
		<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	/// Storage miners the bank distributes the storage emission compartment to.
	///
	/// Implemented by the runtime against the Arion pallet's family/child
	/// registry and its validator-reported node weights, so the bank stays
	/// decoupled from where those weights come from.
	pub trait StorageMinerWeights<AccountId> {
		/// Payout account and weight of every storage miner eligible for the
		/// current payout window. Zero-weight entries receive nothing.
		///
		/// Weights are `u128`, not the `u16` a single Arion *node* weight is
		/// stored as: a payee is a family, and a family's weight is the sum
		/// over its children, which overruns `u16` well inside the runtime's
		/// registration caps.
		///
		/// **Caller contract**: each payout account MUST appear at most once —
		/// the implementation is responsible for aggregating the weights of
		/// every node belonging to the same payee. A repeated account is not
		/// unsound (it just receives several transfers), but it burns one of
		/// the `MaxMinersPerPayout` slots per node instead of per payee.
		///
		/// **Caller contract**: the returned weights MUST sum to strictly less
		/// than `u128::MAX`. The bank folds them into a single `u128`
		/// denominator with `saturating_add`; a sum that saturated would be an
		/// under-sized denominator and the pro-rata shares could then exceed
		/// the requested payout, overdrawing the compartment. The production
		/// implementation inherits this from Arion's per-node `MaxNodeWeight`
		/// cap over a bounded child count — an implementation without an
		/// equivalent per-entry bound must impose one itself.
		///
		/// **Caller contract**: the read cost of this call is NOT covered by
		/// `WeightInfo::pay_storage_miners`'s per-miner term; the
		/// implementation's scan must be bounded by on-chain caps rather than
		/// by anything a caller supplies.
		fn active_storage_miners() -> Vec<(AccountId, u128)>;
	}

	/// Compute miners the bank distributes the compute emission compartment to.
	///
	/// Implemented by the runtime against the compute-scoring pallet's
	/// per-epoch reward weights so the bank stays decoupled from where those
	/// weights come from.
	pub trait ComputeMinerWeights<AccountId> {
		/// Payout account and reward weight of every compute miner eligible
		/// for the current payout window. Zero-weight entries receive nothing.
		///
		/// Weights are `u128` (the compute-scoring `EpochWeights` width).
		///
		/// **Caller contract**: each payout account MUST appear at most once —
		/// the implementation is responsible for aggregating the weights of
		/// every node belonging to the same payee. A repeated account is not
		/// unsound (it just receives several transfers), but it burns one of
		/// the `MaxComputeMinersPerPayout` slots per node instead of per payee.
		///
		/// **Caller contract**: the returned weights MUST sum to strictly less
		/// than `u128::MAX`. The bank folds them into a single `u128`
		/// denominator with `saturating_add`; a sum that saturated would be an
		/// under-sized denominator and the pro-rata shares could then exceed
		/// the requested payout, overdrawing the compartment. The production
		/// implementation inherits this from compute-scoring's per-entry
		/// `MaxEpochWeightPerNode` cap over a bounded entry count — an
		/// implementation without an equivalent per-entry bound must impose
		/// one itself.
		fn active_compute_miners() -> Vec<(AccountId, u128)>;

		/// Identifier of the settlement period the weights above belong to,
		/// used by the bank as an idempotency key.
		///
		/// Unlike the storage weights — a rolling "current standing" that can
		/// legitimately be paid any number of times — compute weights are a
		/// *per-epoch entitlement*: `EpochWeights[epoch]` is written once and
		/// never changes, so paying the same epoch twice pays the same work
		/// twice. The bank refuses to settle an epoch it has already settled.
		///
		/// Return `None` if the source has no settlement period, which opts out
		/// of the replay check entirely.
		fn current_weight_epoch() -> Option<u64>;
	}

	/// Version 1 is "activated": requesters whitelisted and pre-upgrade backing
	/// seeded by `ActivateMinerPaymentBank`. That migration seeds real funds
	/// from sudo, so it keys its one-shot guard on this rather than on
	/// whitelist contents — a requester can be removed by an admin at any time,
	/// and inferring "already ran" from that would re-seed.
	///
	/// Version 2 adds compute settlement (switch off, no keys, no params);
	/// the runtime migration only stamps the version, no balance moves.
	pub const STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

	#[pallet::pallet]
	#[pallet::without_storage_info]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Currency held and paid out by the bank.
		type Currency: Currency<Self::AccountId>;

		/// Pallet id from which the bank sovereign account is derived.
		#[pallet::constant]
		type PalletId: Get<PalletId>;

		/// Origin allowed to manage the requester whitelist.
		type AdminOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		/// Weighted storage miners paid by `pay_storage_miners`.
		type StorageMinerWeights: StorageMinerWeights<Self::AccountId>;

		/// Most storage miners a single `pay_storage_miners` call will pay.
		#[pallet::constant]
		type MaxMinersPerPayout: Get<u32>;

		/// Blocks in a 24-hour period for rate limiting miner payouts.
		#[pallet::constant]
		type BlocksPer24Hours: Get<BlockNumberFor<Self>>;

		/// Maximum emission to distribute to miners per 24-hour period (in
		/// planck). A single budget shared by `pay_storage_miners` and
		/// `pay_compute_miners`: what one spends the other cannot.
		#[pallet::constant]
		type Max24HourMinerPayout: Get<BalanceOf<Self>>;

		/// Weighted compute miners paid by `pay_compute_miners`.
		type ComputeMinerWeights: ComputeMinerWeights<Self::AccountId>;

		/// Most compute miners a single `pay_compute_miners` call will pay.
		#[pallet::constant]
		type MaxComputeMinersPerPayout: Get<u32>;

		type WeightInfo: WeightInfo;

		/// Value-backed compute revenue per source window (the marketplace).
		type ComputeRevenue: st::ComputeRevenueSource;

		/// The chain's alpha price (`Credits.AlphaPrice`).
		type AlphaPrice: st::AlphaPriceSource<BlockNumberFor<Self>>;

		/// Oldest `AlphaPrice` an approval accepts, in blocks.
		#[pallet::constant]
		type MaxAlphaPriceAge: Get<BlockNumberFor<Self>>;

		/// Roles a settlement key must not also hold.
		type SettlementKeyRoles: st::SettlementKeyRoles<Self::AccountId>;

		/// Origin that schedules settlement changes (root).
		type SettlementRootOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		/// Blocks between scheduling a settlement change and applying it.
		#[pallet::constant]
		type RootChangeDelay: Get<BlockNumberFor<Self>>;

		/// Most entries one `pay_compute_part` may carry.
		#[pallet::constant]
		type MaxEntriesPerPart: Get<u32>;

		/// Windows after which a window can no longer be revised (160).
		#[pallet::constant]
		type ComputeRevisionHorizon: Get<u32>;
	}

	/// Source of deposited funds.
	#[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum DepositType {
		/// Revenue collected through the marketplace (credits deposits,
		/// subscriptions) — storage and compute are not distinguishable there.
		MarketplaceRevenue,
		/// Protocol emissions.
		Emission,
		/// One-off grant / treasury top-up.
		Grant,
		/// Transaction fees collected from the network.
		Fees,
		/// Anything else.
		Other,
		/// Protocol emissions earmarked for compute miners. A separate
		/// compartment from [`DepositType::Emission`] (which backs
		/// `pay_storage_miners`): each is spendable only by its own payout.
		///
		/// Appended last on purpose — `DepositType` is a storage key, and the
		/// existing variants must keep their SCALE discriminants.
		ComputeEmission,
	}

	/// Global payout switch (defaults to enabled). While `false`,
	/// `request_payment` rejects every caller with
	/// [`Error::DistributionDisabled`]; deposits are unaffected.
	#[pallet::storage]
	#[pallet::getter(fn distribution_enabled)]
	pub type DistributionEnabled<T: Config> =
		StorageValue<_, bool, ValueQuery, frame_support::traits::ConstBool<true>>;

	/// Accounts allowed to call `request_payment`.
	#[pallet::storage]
	pub type WhitelistedRequesters<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	/// Accounts allowed to call `pay_storage_miners`.
	#[pallet::storage]
	pub type MinerPaymentWhitelist<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	/// Lifetime total deposited, per deposit type.
	#[pallet::storage]
	pub type TotalDeposited<T: Config> =
		StorageMap<_, Blake2_128Concat, DepositType, BalanceOf<T>, ValueQuery>;

	/// Lifetime total released through `request_payment` and `pay_storage_miners`.
	#[pallet::storage]
	pub type TotalPaidOut<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Lifetime total distributed to storage miners out of the emission
	/// compartment. `TotalDeposited[Emission] - EmissionPaidOut` is the
	/// compartment balance still reserved for `pay_storage_miners`.
	#[pallet::storage]
	pub type EmissionPaidOut<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Amount distributed to miners in the current 24-hour period — storage and
	/// compute payouts draw on this **one** counter, bounded by
	/// `Max24HourMinerPayout`. Resets every 24 hours.
	#[pallet::storage]
	pub type MinerPayoutPeriodAmount<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Block number when the current 24-hour payout period started.
	#[pallet::storage]
	pub type MinerPayoutPeriodStart<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

	/// Lifetime total distributed to compute miners out of the compute
	/// emission compartment. `TotalDeposited[ComputeEmission] -
	/// ComputeEmissionPaidOut` is the compartment balance still reserved for
	/// `pay_compute_miners`.
	#[pallet::storage]
	pub type ComputeEmissionPaidOut<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Last compute epoch settled by `pay_compute_miners`, the idempotency key
	/// that stops one epoch's weights being paid twice.
	///
	/// `OptionQuery`: `None` means nothing has ever been settled, which is
	/// distinct from having settled epoch `0`. With `ValueQuery` those two
	/// states collide and a genuine epoch-0 payout could be replayed.
	#[pallet::storage]
	pub type LastPaidComputeEpoch<T: Config> = StorageValue<_, u64, OptionQuery>;

	/// Lifetime total released per requester (e.g. arion vs compute escrow).
	#[pallet::storage]
	pub type TotalPaidByRequester<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

	// ---------------------------------------------------------------------
	// Compute settlement
	// ---------------------------------------------------------------------

	/// Settlement switch, off by default. Root enables it through a delayed
	/// change; the admin or root disables it at once.
	#[pallet::storage]
	pub type ComputeSettlementEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	/// Accounts that submit approvals and parts (vali).
	#[pallet::storage]
	pub type ComputeSettlementSubmitters<T: Config> =
		StorageValue<_, BoundedVec<T::AccountId, ConstU32<MAX_SETTLEMENT_KEYS>>, ValueQuery>;

	/// sr25519 keys whose signature an approval needs (backend).
	#[pallet::storage]
	pub type ComputeSettlementApproverKeys<T: Config> =
		StorageValue<_, BoundedVec<sr25519::Public, ConstU32<MAX_SETTLEMENT_KEYS>>, ValueQuery>;

	/// The payout params in force: `(version, ParamsV1)`, versions from 1.
	/// Revision 0 must sign this version.
	#[pallet::storage]
	pub type ComputePayoutParams<T: Config> =
		StorageValue<_, (u32, st::PayoutParams), OptionQuery>;

	/// The approval being paid, if any: one at a time across all windows.
	#[pallet::storage]
	pub type InFlightApproval<T: Config> = StorageValue<_, (u32, u8), OptionQuery>;

	/// Global completion order of approvals, `(window, revision)` → seq
	/// (the backend nets debts in this order). Present = completed.
	#[pallet::storage]
	pub type ComputeRevisionSeq<T: Config> =
		StorageMap<_, Twox64Concat, (u32, u8), u64, OptionQuery>;

	/// The next completion sequence number.
	#[pallet::storage]
	pub type NextRevisionSeq<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// UTC day (`current_compute_period / 24`) the window's latest revision
	/// completed; revision `k > 0` needs it before today.
	#[pallet::storage]
	pub type ComputeLastRevisionDay<T: Config> =
		StorageMap<_, Twox64Concat, u32, u32, OptionQuery>;

	/// Most alpha settlement may pay per UTC day. Zero (the default) pays
	/// nothing.
	#[pallet::storage]
	pub type ComputeMaxDailyAlpha<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Root changes waiting for their delay: `(change, effective block)`.
	#[pallet::storage]
	pub type PendingRootChange<T: Config> = StorageMap<
		_,
		Twox64Concat,
		st::ChangeKind,
		(st::RootChange<T::AccountId>, BlockNumberFor<T>),
		OptionQuery,
	>;

	/// First window settlement may pay; set once.
	#[pallet::storage]
	pub type FirstComputeWindow<T: Config> = StorageValue<_, u32, OptionQuery>;

	/// The window revision 0 must settle next.
	#[pallet::storage]
	pub type NextComputeWindow<T: Config> = StorageValue<_, u32, ValueQuery>;

	/// Approvals by `(window, revision)`.
	#[pallet::storage]
	pub type ComputeApproval<T: Config> =
		StorageMap<_, Twox64Concat, (u32, u8), st::Approval, OptionQuery>;

	/// Payment progress by `(window, revision)`.
	#[pallet::storage]
	pub type ComputeProgress<T: Config> =
		StorageMap<_, Twox64Concat, (u32, u8), st::Progress<T::AccountId>, OptionQuery>;

	/// `(params_version, ParamsV1, formula_version)` pinned by revision 0 of
	/// the window; later revisions must match it and use its share.
	#[pallet::storage]
	pub type WindowParams<T: Config> =
		StorageMap<_, Twox64Concat, u32, (u32, st::PayoutParams, u16), OptionQuery>;

	/// Paid per window by completed revisions: `(usd credits, alpha)`.
	#[pallet::storage]
	pub type ComputeWindowPaid<T: Config> =
		StorageMap<_, Twox64Concat, u32, (u128, u128), ValueQuery>;

	/// Alpha whose transfer failed, claimable by the family.
	#[pallet::storage]
	pub type ComputeOwed<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

	/// Σ `ComputeOwed`.
	#[pallet::storage]
	pub type ComputeOwedTotal<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Alpha paid by settlement on a UTC day: `(day, alpha)`.
	#[pallet::storage]
	pub type ComputeDailyAlpha<T: Config> = StorageValue<_, (u64, u128), ValueQuery>;

	/// Alpha reserved by the approval in flight and not yet paid:
	/// `floor(usd × 1e18 / price)` at approval, less each part paid; the rest
	/// is released at completion or revoke.
	#[pallet::storage]
	pub type ApprovedUnpaidAlpha<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Where `Defund` pays to; set by root.
	#[pallet::storage]
	pub type ComputeRevenueAccount<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

	/// Accounts allowed to draw from the bank at genesis.
	///
	/// A genesis-built chain never runs `ActivateMinerPaymentBank` — FRAME's
	/// `on_genesis` stamps the pallet's storage version, so the migration's
	/// one-shot guard sees an already-current chain and does nothing. New chains
	/// must therefore whitelist here or no payment ever succeeds; the migration
	/// exists only to carry an already-running chain across the upgrade.
	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub requesters: Vec<T::AccountId>,
	}

	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self { requesters: Vec::new() }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			for who in &self.requesters {
				WhitelistedRequesters::<T>::insert(who, ());
			}
		}
	}

	/// Largest amount a single `request_payment` call from this requester may
	/// move. Absent means uncapped.
	///
	/// This is a per-call bound, **not** a spend limit: a requester that calls
	/// repeatedly still withdraws without limit, and arion calls once per
	/// family per settlement. Treat it as a blast-radius limiter on one bad
	/// request, not as compartmentalization — the wall that keeps money owed
	/// to others out of a consumer's budget lives with each consumer: the
	/// runtime's `PayoutSource` adapter for arion, and the marketplace's
	/// referral-commission payout for the marketplace.
	#[pallet::storage]
	pub type RequesterWithdrawalCap<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Funds were deposited into the bank.
		Deposited { who: T::AccountId, amount: BalanceOf<T>, deposit_type: DepositType },
		/// An account was added to the requester whitelist.
		RequesterAdded { who: T::AccountId },
		/// An account was removed from the requester whitelist.
		RequesterRemoved { who: T::AccountId },
		/// A payment was released to `dest`. `paid` may be lower than `requested`
		/// when the bank balance is insufficient.
		PaymentReleased {
			requester: T::AccountId,
			dest: T::AccountId,
			requested: BalanceOf<T>,
			paid: BalanceOf<T>,
		},
		/// Per-requester withdrawal cap was set.
		RequesterCapSet { who: T::AccountId, cap: BalanceOf<T> },
		/// Per-requester withdrawal cap was removed.
		RequesterCapRemoved { who: T::AccountId },
		/// Distribution enabled/disabled status changed.
		DistributionEnabledChanged { enabled: bool },
		/// `pay_storage_miners` distributed emission pro-rata by family weight.
		/// `paid < requested` when shares rounded to zero or a transfer was
		/// skipped; the difference stays in the emission compartment.
		StorageMinersPaid {
			requested: BalanceOf<T>,
			paid: BalanceOf<T>,
			miners_paid: u32,
			miners_skipped: u32,
		},
		/// Individual storage miner payment detail for indexing.
		MinerPaymentPaid { miner: T::AccountId, amount: BalanceOf<T> },
		/// An account was added to the miner payment whitelist.
		MinerPaymentCallerAdded { who: T::AccountId },
		/// An account was removed from the miner payment whitelist.
		MinerPaymentCallerRemoved { who: T::AccountId },
		/// `pay_compute_miners` distributed compute emission pro-rata by epoch
		/// reward weight. `paid < requested` when shares rounded to zero or a
		/// transfer was skipped; the difference stays in the compartment.
		ComputeMinersPaid {
			requested: BalanceOf<T>,
			paid: BalanceOf<T>,
			miners_paid: u32,
			miners_skipped: u32,
		},
		/// Individual compute miner payment detail for indexing.
		ComputeMinerPaymentPaid { miner: T::AccountId, amount: BalanceOf<T> },
		/// A settlement approval was accepted.
		ComputeApprovalSubmitted {
			window: u32,
			revision: u8,
			parts: u8,
			usd: u128,
			price: u128,
			params_version: u32,
			formula_version: u16,
			payable_root: H256,
			evidence_root: H256,
		},
		/// An approval with nothing paid yet was revoked.
		ComputeApprovalRevoked { window: u32, revision: u8 },
		/// A part was paid.
		ComputePartPaid { window: u32, revision: u8, part: u8, usd: u128, alpha: u128 },
		/// Every part of an approval was paid.
		ComputeWindowCompleted { window: u32, revision: u8, usd: u128, alpha: u128 },
		/// A family was paid.
		ComputeMinerSettled { family: T::AccountId, alpha: BalanceOf<T> },
		/// A transfer to a family failed; the amount is owed to it.
		ComputeOwedRecorded { family: T::AccountId, alpha: BalanceOf<T> },
		/// A family claimed what it was owed.
		ComputeOwedClaimed { family: T::AccountId, alpha: BalanceOf<T> },
		/// The settlement account was funded.
		ComputeSettlementFunded { who: T::AccountId, amount: BalanceOf<T> },
		/// Unreserved settlement funds went back to the revenue account.
		ComputeSettlementDefunded { to: T::AccountId, amount: BalanceOf<T> },
		/// A root change was scheduled.
		RootChangeScheduled { kind: st::ChangeKind, effective: BlockNumberFor<T> },
		/// A root change was applied.
		RootChangeApplied { kind: st::ChangeKind },
		/// A pending root change was cancelled.
		RootChangeCancelled { kind: st::ChangeKind },
		/// The settlement switch changed.
		ComputeSettlementEnabledChanged { enabled: bool },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The caller of `request_payment` is not whitelisted.
		RequesterNotWhitelisted,
		/// Amount must be non-zero.
		ZeroAmount,
		/// Account is already whitelisted.
		AlreadyWhitelisted,
		/// Account is not in the whitelist.
		NotWhitelisted,
		/// Distributions are currently disabled.
		DistributionDisabled,
		/// `pay_storage_miners` asked for more than the emission compartment holds.
		InsufficientEmissionFunds,
		/// `pay_storage_miners` asked for more than the bank can physically pay.
		InsufficientBankBalance,
		/// The ranked-miner list exceeds `MaxMinersPerPayout`.
		TooManyMiners,
		/// No ranked storage miner carries a non-zero weight.
		NoEligibleMiners,
		/// Caller is not whitelisted to call `pay_storage_miners`.
		PaymentCallerNotWhitelisted,
		/// The shared 24-hour miner payout limit would be exceeded. Raised by
		/// both `pay_storage_miners` and `pay_compute_miners`.
		ExceedsDaily24HourMinerPayoutLimit,
		/// `pay_compute_miners` asked for more than the compute emission
		/// compartment holds.
		InsufficientComputeEmissionFunds,
		/// The weighted compute-miner list exceeds `MaxComputeMinersPerPayout`.
		TooManyComputeMiners,
		/// No compute miner carries a non-zero weight this window.
		NoEligibleComputeMiners,
		/// This compute epoch has already been settled. Re-paying it would
		/// pay the same epoch's work twice; wait for the next epoch close.
		ComputeEpochAlreadyPaid,
		/// The weight source returned a set whose weights sum past `u128`.
		/// Both payouts reject rather than settle: a saturated denominator
		/// inflates every pro-rata share and can overdraw the compartment.
		/// Appended last on purpose — `Error` variants are metadata-indexed.
		WeightSumOverflow,
		/// `ComputeSettlementEnabled` is off.
		ComputeSettlementDisabled,
		/// The caller is not a settlement submitter.
		NotComputeSubmitter,
		/// Submitters or approver keys are empty, repeated or overlap.
		InvalidSettlementKeys,
		/// A settlement key also holds another privileged role.
		SettlementKeyHasOtherRole,
		/// The backend signature does not verify under any approver key.
		BadApproverSignature,
		/// No first window has been set.
		FirstWindowNotSet,
		/// The first window is already set.
		FirstWindowAlreadySet,
		/// The window is before the first window.
		WindowBeforeFirst,
		/// Revision 0 is not for `NextComputeWindow`.
		WindowOutOfOrder,
		/// Revision `k` needs revision `k − 1` completed.
		RevisionOutOfOrder,
		/// Revision `k − 1` completed on the same chain day.
		RevisionSameDay,
		/// The window is past its revision horizon.
		RevisionTooLate,
		/// An approval already exists for this `(window, revision)`.
		ApprovalExists,
		/// No approval for this `(window, revision)`.
		NoApproval,
		/// `parts`, `part_hashes` and `usd` disagree.
		MalformedApproval,
		/// Params version, formula or params hash differ from what is required.
		ParamsMismatch,
		/// Invalid payout params.
		InvalidParams,
		/// No params set.
		ParamsNotSet,
		/// The window's revenue is not final yet.
		RevenueNotFinal,
		/// The approval exceeds `(r + tol) × charged(W)`.
		RevenueCapExceeded,
		/// `AlphaPrice` is unset or older than `MaxAlphaPriceAge`.
		AlphaPriceStale,
		/// The signed price differs from the chain's `AlphaPrice`.
		AlphaPriceMismatch,
		/// Alpha conversion overflowed.
		AlphaOverflow,
		/// The part is not the next one.
		PartOutOfOrder,
		/// The part does not hash to the approved part hash.
		PartHashMismatch,
		/// The part has no entries.
		EmptyPart,
		/// Families are not strictly ascending within and across parts.
		FamiliesNotAscending,
		/// An entry has zero USD.
		ZeroUsd,
		/// An entry's alpha differs from `floor(usd × 1e18 / price)`.
		AlphaMismatch,
		/// Paid USD would exceed the approval.
		TotalsExceeded,
		/// The last part leaves paid USD unequal to the approval.
		TotalsMismatch,
		/// The settlement account cannot cover the part above its seed.
		InsufficientSettlementFunds,
		/// The part would exceed the daily alpha cap.
		DailyAlphaCapExceeded,
		/// A failed payment could not be moved to the owed account.
		OwedTransferFailed,
		/// Nothing is owed to the caller.
		NothingOwed,
		/// The approval is already (partly) paid or complete.
		ApprovalAlreadyStarted,
		/// No such pending change.
		NoPendingChange,
		/// The pending change's delay has not passed.
		ChangeNotDue,
		/// No revenue account set for `Defund`.
		NoRevenueAccount,
		/// Nothing can be defunded.
		NothingToDefund,
		/// A part other than the last does not carry `MaxEntriesPerPart`
		/// entries.
		NonCanonicalPart,
		/// Another approval is being paid; one at a time.
		ApprovalInFlight,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Deposit funds into the bank, tagged with their source type.
		#[pallet::call_index(0)]
		#[pallet::weight(<T as Config>::WeightInfo::deposit())]
		pub fn deposit(
			origin: OriginFor<T>,
			amount: BalanceOf<T>,
			deposit_type: DepositType,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			Self::deposit_from(&who, amount, deposit_type)
		}

		/// Add an account to the requester whitelist.
		#[pallet::call_index(1)]
		#[pallet::weight(<T as Config>::WeightInfo::add_requester())]
		pub fn add_requester(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(
				!WhitelistedRequesters::<T>::contains_key(&who),
				Error::<T>::AlreadyWhitelisted
			);
			WhitelistedRequesters::<T>::insert(&who, ());
			Self::deposit_event(Event::RequesterAdded { who });
			Ok(())
		}

		/// Remove an account from the requester whitelist.
		#[pallet::call_index(2)]
		#[pallet::weight(<T as Config>::WeightInfo::remove_requester())]
		pub fn remove_requester(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(WhitelistedRequesters::<T>::contains_key(&who), Error::<T>::NotWhitelisted);
			WhitelistedRequesters::<T>::remove(&who);
			Self::deposit_event(Event::RequesterRemoved { who });
			Ok(())
		}

		#[pallet::call_index(3)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_requester_cap(
			origin: OriginFor<T>,
			who: T::AccountId,
			cap: BalanceOf<T>,
		) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			RequesterWithdrawalCap::<T>::insert(&who, cap);
			Self::deposit_event(Event::RequesterCapSet { who, cap });
			Ok(())
		}

		#[pallet::call_index(4)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn remove_requester_cap(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			RequesterWithdrawalCap::<T>::remove(&who);
			Self::deposit_event(Event::RequesterCapRemoved { who });
			Ok(())
		}

		/// Globally enable or disable bank payouts. While disabled,
		/// `request_payment` rejects every caller with
		/// [`Error::DistributionDisabled`]; each caller decides how to handle
		/// the rejection (retry, arrears, skip). Deposits are unaffected.
		#[pallet::call_index(5)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_distribution_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			DistributionEnabled::<T>::put(enabled);
			Self::deposit_event(Event::DistributionEnabledChanged { enabled });
			Ok(())
		}

		/// Add an account to the miner payment whitelist (callers of `pay_storage_miners`).
		#[pallet::call_index(6)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn add_miner_payment_caller(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(
				!MinerPaymentWhitelist::<T>::contains_key(&who),
				Error::<T>::AlreadyWhitelisted
			);
			MinerPaymentWhitelist::<T>::insert(&who, ());
			Self::deposit_event(Event::MinerPaymentCallerAdded { who });
			Ok(())
		}

		/// Remove an account from the miner payment whitelist.
		#[pallet::call_index(7)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn remove_miner_payment_caller(
			origin: OriginFor<T>,
			who: T::AccountId,
		) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(MinerPaymentWhitelist::<T>::contains_key(&who), Error::<T>::NotWhitelisted);
			MinerPaymentWhitelist::<T>::remove(&who);
			Self::deposit_event(Event::MinerPaymentCallerRemoved { who });
			Ok(())
		}

		/// Distribute `amount` from the bank's emission compartment to storage
		/// miners, pro-rata to their weight.
		///
		/// Callable only by whitelisted callers. The runtime's
		/// `StorageMinerWeights` implementation decides who is eligible, which
		/// account each payee is paid at, and what that payee's weight is —
		/// in production, an Arion family and the summed node weight of its
		/// eligible children. Floor-division dust and skipped shares stay in
		/// the compartment.
		#[pallet::call_index(8)]
		#[pallet::weight(<T as Config>::WeightInfo::pay_storage_miners(T::MaxMinersPerPayout::get()))]
		pub fn pay_storage_miners(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(
				MinerPaymentWhitelist::<T>::contains_key(&who),
				Error::<T>::PaymentCallerNotWhitelisted
			);
			ensure!(DistributionEnabled::<T>::get(), Error::<T>::DistributionDisabled);
			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);

			// 24-hour rate limit: reset period if needed and check limit.
			let current_block = <frame_system::Pallet<T>>::block_number();
			let period_start = MinerPayoutPeriodStart::<T>::get();
			let blocks_per_24h = T::BlocksPer24Hours::get();
			if current_block.saturating_sub(period_start) >= blocks_per_24h {
				// Reset period: 24 hours have passed
				MinerPayoutPeriodStart::<T>::put(current_block);
				MinerPayoutPeriodAmount::<T>::put(BalanceOf::<T>::zero());
			}

			// Check if this payout would exceed the 24-hour limit
			let current_period_amount = MinerPayoutPeriodAmount::<T>::get();
			let max_per_24h = T::Max24HourMinerPayout::get();
			let new_period_amount = current_period_amount.saturating_add(amount);
			ensure!(
				new_period_amount <= max_per_24h,
				Error::<T>::ExceedsDaily24HourMinerPayoutLimit
			);

			// Compartment wall: only bridged emission funds this payout — never
			// marketplace backing, fees, or grants. Rejecting (not clamping)
			// keeps "pay_storage_miners(X) pays exactly X" honest.
			ensure!(amount <= Self::emission_available(), Error::<T>::InsufficientEmissionFunds);
			// The compartment ledger can exceed what is physically free (another
			// consumer overdrew, or the ED cushion); never overdraw the account.
			ensure!(amount <= Self::available_for_payout(), Error::<T>::InsufficientBankBalance);

			let miners = T::StorageMinerWeights::active_storage_miners();
			ensure!(
				u32::try_from(miners.len()).unwrap_or(u32::MAX) <= T::MaxMinersPerPayout::get(),
				Error::<T>::TooManyMiners
			);
			// `checked_add`, not `sum()` and not `saturating_add`. A plain sum
			// wraps in release; saturating clamps. BOTH produce a denominator
			// that is too *small*, and a small denominator inflates every
			// pro-rata share — `weight_share` caps one share at the pool, but
			// nothing caps their sum, and the compartment guards above were
			// checked once against `amount` before this loop. Two entries at
			// `u128::MAX` saturate to `u128::MAX` and then pay each of them
			// the FULL requested amount, walking other compartments' funds out
			// of the bank booked as emission. The trait forbids such a set;
			// this refuses to settle one rather than trusting the contract.
			let total_weight: u128 = miners
				.iter()
				.try_fold(0u128, |acc, (_, w)| acc.checked_add(*w))
				.ok_or(Error::<T>::WeightSumOverflow)?;
			ensure!(total_weight > 0, Error::<T>::NoEligibleMiners);

			let bank = Self::account_id();
			let pool = payment_math::Tokens::new(amount.saturated_into());
			let mut paid: BalanceOf<T> = Zero::zero();
			let mut miners_paid: u32 = 0;
			let mut miners_skipped: u32 = 0;

			for (owner, weight) in miners {
				let share: BalanceOf<T> =
					payment_math::weight_share(pool, weight, total_weight).get().saturated_into();
				// Zero shares (weight rounds below one planck) and failed
				// transfers (e.g. a reaped owner account below its ED) are
				// skipped, not fatal: their share stays in the compartment.
				if share.is_zero() {
					miners_skipped = miners_skipped.saturating_add(1);
					continue;
				}
				match T::Currency::transfer(&bank, &owner, share, ExistenceRequirement::KeepAlive) {
					Ok(()) => {
						paid = paid.saturating_add(share);
						miners_paid = miners_paid.saturating_add(1);
						// Emit event for individual miner payment (for indexing)
						Self::deposit_event(Event::MinerPaymentPaid {
							miner: owner.clone(),
							amount: share,
						});
					},
					Err(_) => miners_skipped = miners_skipped.saturating_add(1),
				}
			}

			EmissionPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
			TotalPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
			// Update the 24-hour period amount
			MinerPayoutPeriodAmount::<T>::mutate(|t| *t = t.saturating_add(paid));
			Self::deposit_event(Event::StorageMinersPaid {
				requested: amount,
				paid,
				miners_paid,
				miners_skipped,
			});
			Ok(())
		}

		/// Distribute `amount` from the bank's **compute** emission compartment
		/// to compute miners, pro-rata to their epoch reward weight.
		///
		/// Callable only by whitelisted miner-payment callers — the same
		/// whitelist that gates `pay_storage_miners`, since both are driven by
		/// the same admin-run payout key. The runtime's `ComputeMinerWeights`
		/// implementation decides which nodes are eligible and which account
		/// each node pays out to. Floor-division dust and skipped shares stay
		/// in the compartment.
		///
		/// Shares the 24-hour `Max24HourMinerPayout` budget with
		/// `pay_storage_miners` — the two compartments are separate pots, but
		/// the daily drain rate is capped once across both.
		#[pallet::call_index(9)]
		#[pallet::weight(<T as Config>::WeightInfo::pay_compute_miners(T::MaxComputeMinersPerPayout::get()))]
		pub fn pay_compute_miners(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(
				MinerPaymentWhitelist::<T>::contains_key(&who),
				Error::<T>::PaymentCallerNotWhitelisted
			);
			ensure!(DistributionEnabled::<T>::get(), Error::<T>::DistributionDisabled);
			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);

			// 24-hour rate limit — the SAME counter `pay_storage_miners` uses.
			// `Max24HourMinerPayout` bounds total emission leaving the bank per
			// day across both payouts, so what storage spends compute cannot,
			// and vice versa. (The compartments below stay separate: a shared
			// rate limit governs the pace, not the entitlement.)
			let current_block = <frame_system::Pallet<T>>::block_number();
			let period_start = MinerPayoutPeriodStart::<T>::get();
			let blocks_per_24h = T::BlocksPer24Hours::get();
			if current_block.saturating_sub(period_start) >= blocks_per_24h {
				MinerPayoutPeriodStart::<T>::put(current_block);
				MinerPayoutPeriodAmount::<T>::put(BalanceOf::<T>::zero());
			}

			let current_period_amount = MinerPayoutPeriodAmount::<T>::get();
			let max_per_24h = T::Max24HourMinerPayout::get();
			let new_period_amount = current_period_amount.saturating_add(amount);
			ensure!(
				new_period_amount <= max_per_24h,
				Error::<T>::ExceedsDaily24HourMinerPayoutLimit
			);

			// Compartment wall: only funds deposited as `ComputeEmission` pay
			// compute miners — never storage emission, marketplace backing,
			// fees, or grants.
			ensure!(
				amount <= Self::compute_emission_available(),
				Error::<T>::InsufficientComputeEmissionFunds
			);
			// The compartment ledger can exceed what is physically free (another
			// consumer overdrew, or the ED cushion); never overdraw the account.
			ensure!(amount <= Self::available_for_payout(), Error::<T>::InsufficientBankBalance);

			// Replay guard. `EpochWeights[epoch]` is written once by the epoch
			// close and never mutates, so re-running this call against the same
			// epoch pays the same work a second time — a retried transaction
			// (submitter didn't see the receipt) silently double-spends the
			// compartment. Settle each epoch at most once, strictly forward.
			let weight_epoch = T::ComputeMinerWeights::current_weight_epoch();
			if let Some(epoch) = weight_epoch {
				if let Some(last_paid) = LastPaidComputeEpoch::<T>::get() {
					ensure!(epoch > last_paid, Error::<T>::ComputeEpochAlreadyPaid);
				}
			}

			let miners = T::ComputeMinerWeights::active_compute_miners();
			ensure!(
				u32::try_from(miners.len()).unwrap_or(u32::MAX)
					<= T::MaxComputeMinersPerPayout::get(),
				Error::<T>::TooManyComputeMiners
			);
			// `saturating_add` here is an overflow guard, NOT a benign
			// rounding choice: a total that actually saturated would be an
			// UNDER-sized denominator, and `Σ weight_share(pool, wᵢ, total)`
			// could then exceed `pool` — overdrawing the compartment and
			// booking past the daily cap. (Two payees at `u128::MAX` would
			// each receive the whole pool.) So conservation depends on the
			// fold never saturating, which holds because the compute-scoring
			// pallet caps every entry at `MaxEpochWeightPerNode` (default
			// `u64::MAX`) before writing it and at most
			// `MaxMinerStatusUpdatesPerCall` entries exist per epoch —
			// a ceiling ~17 orders of magnitude below `u128::MAX`.
			//
			// That margin lives in ANOTHER pallet's constant. Rather than trust
			// it, the fold refuses to settle a set that overruns `u128` at all
			// — so raising `MaxEpochWeightPerNode`, or supplying a
			// `ComputeMinerWeights` implementation that does not inherit the
			// per-entry cap, fails the payout instead of silently overdrawing.
			let total_weight: u128 = miners
				.iter()
				.try_fold(0u128, |acc, (_, w)| acc.checked_add(*w))
				.ok_or(Error::<T>::WeightSumOverflow)?;
			ensure!(total_weight > 0, Error::<T>::NoEligibleComputeMiners);

			let bank = Self::account_id();
			let pool = payment_math::Tokens::new(amount.saturated_into());
			let mut paid: BalanceOf<T> = Zero::zero();
			let mut miners_paid: u32 = 0;
			let mut miners_skipped: u32 = 0;

			for (owner, weight) in miners {
				let share: BalanceOf<T> =
					payment_math::weight_share(pool, weight, total_weight).get().saturated_into();
				// Zero shares (weight rounds below one planck) and failed
				// transfers (e.g. a reaped owner account below its ED) are
				// skipped, not fatal: their share stays in the compartment.
				if share.is_zero() {
					miners_skipped = miners_skipped.saturating_add(1);
					continue;
				}
				match T::Currency::transfer(&bank, &owner, share, ExistenceRequirement::KeepAlive) {
					Ok(()) => {
						paid = paid.saturating_add(share);
						miners_paid = miners_paid.saturating_add(1);
						Self::deposit_event(Event::ComputeMinerPaymentPaid {
							miner: owner.clone(),
							amount: share,
						});
					},
					Err(_) => miners_skipped = miners_skipped.saturating_add(1),
				}
			}

			ComputeEmissionPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
			TotalPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
			MinerPayoutPeriodAmount::<T>::mutate(|t| *t = t.saturating_add(paid));
			// Advance the cursor only once something actually moved. A call
			// where every transfer was skipped (e.g. all owners sat below their
			// ED) paid nobody, and burning the epoch here would strand it
			// permanently — the operator could never retry after fixing the
			// cause. A PARTIAL payout does consume the epoch, matching the
			// existing rule that skipped shares and floor-division dust stay in
			// the compartment rather than being re-paid.
			if !paid.is_zero() {
				if let Some(epoch) = weight_epoch {
					LastPaidComputeEpoch::<T>::put(epoch);
				}
			}
			Self::deposit_event(Event::ComputeMinersPaid {
				requested: amount,
				paid,
				miners_paid,
				miners_skipped,
			});
			Ok(())
		}

		/// Accept the backend-signed approval of `(window, revision)`.
		///
		/// Origin: a submitter. `backend_sig` is an sr25519 signature by an
		/// approver key over [`st::approval_digest`]. Revision 0 settles
		/// `NextComputeWindow`; revision `k` pays escrowed hours of an earlier
		/// window, at most one per chain day, until the revision horizon.
		/// With `parts = 0` the approval completes at once. Feeless on
		/// success.
		#[pallet::call_index(10)]
		#[pallet::weight(<T as Config>::WeightInfo::submit_compute_approval())]
		pub fn submit_compute_approval(
			origin: OriginFor<T>,
			window: u32,
			revision: u8,
			input: st::ApprovalInput,
			backend_sig: sr25519::Signature,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
			Self::ensure_switches()?;
			Self::ensure_submitter(&who)?;
			Self::ensure_current_keys_valid()?;

			let digest = st::approval_digest(&Self::genesis_hash(), window, revision, &input);
			Self::ensure_approver_signature(&digest, &backend_sig)?;

			ensure!(InFlightApproval::<T>::get().is_none(), Error::<T>::ApprovalInFlight);
			ensure!(
				!ComputeApproval::<T>::contains_key((window, revision)),
				Error::<T>::ApprovalExists
			);
			ensure!(
				input.part_hashes.len() == usize::from(input.parts)
					&& ((input.parts == 0) == (input.usd == 0)),
				Error::<T>::MalformedApproval
			);
			let first = FirstComputeWindow::<T>::get().ok_or(Error::<T>::FirstWindowNotSet)?;
			ensure!(window >= first, Error::<T>::WindowBeforeFirst);

			let today = T::ComputeRevenue::current_window();
			let params = Self::check_order_and_params(window, revision, today, &input)?;

			let charged =
				T::ComputeRevenue::final_charged(window).ok_or(Error::<T>::RevenueNotFinal)?;
			let (paid_usd, _) = ComputeWindowPaid::<T>::get(window);
			ensure!(
				st::within_revenue_cap(
					paid_usd,
					input.usd,
					charged,
					params.share_bps,
					params.tol_bps
				),
				Error::<T>::RevenueCapExceeded
			);

			let price = Self::fresh_alpha_price()?;
			ensure!(input.price == price, Error::<T>::AlphaPriceMismatch);
			let alpha = st::alpha_for(input.usd, price).ok_or(Error::<T>::AlphaOverflow)?;

			if revision == 0 {
				WindowParams::<T>::insert(
					window,
					(input.params_version, params, input.formula_version),
				);
			}
			InFlightApproval::<T>::put((window, revision));
			let event = Event::ComputeApprovalSubmitted {
				window,
				revision,
				parts: input.parts,
				usd: input.usd,
				price,
				params_version: input.params_version,
				formula_version: input.formula_version,
				payable_root: input.payable_root,
				evidence_root: input.evidence_root,
			};
			let parts = input.parts;
			ComputeApproval::<T>::insert(
				(window, revision),
				st::Approval { input, approval_digest: digest },
			);
			ComputeProgress::<T>::insert(
				(window, revision),
				st::Progress {
					next_part: 0,
					last_family: Self::zero_account(),
					usd_paid: 0,
					alpha_paid: 0,
				},
			);
			ApprovedUnpaidAlpha::<T>::mutate(|r| *r = r.saturating_add(alpha));
			Self::deposit_event(event);

			if parts == 0 {
				Self::complete_approval(window, revision, today);
			}
			Ok(Pays::No.into())
		}

		/// Pay part `part` of the approval of `(window, revision)`.
		///
		/// Origin: a submitter. Every check runs before any transfer; the
		/// call is one storage transaction. A transfer that fails is moved to
		/// the owed account and claimable once by the family. Feeless on
		/// success.
		#[pallet::call_index(11)]
		#[pallet::weight(<T as Config>::WeightInfo::pay_compute_part(T::MaxEntriesPerPart::get()))]
		pub fn pay_compute_part(
			origin: OriginFor<T>,
			window: u32,
			revision: u8,
			part: u8,
			entries: BoundedVec<st::Entry<T::AccountId>, T::MaxEntriesPerPart>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
			Self::ensure_switches()?;
			Self::ensure_submitter(&who)?;

			let approval = ComputeApproval::<T>::get((window, revision))
				.ok_or(Error::<T>::NoApproval)?
				.input;
			let mut progress =
				ComputeProgress::<T>::get((window, revision)).ok_or(Error::<T>::NoApproval)?;
			// A completed approval has `next_part == parts`.
			ensure!(
				part == progress.next_part && part < approval.parts,
				Error::<T>::PartOutOfOrder
			);
			ensure!(!entries.is_empty(), Error::<T>::EmptyPart);
			let is_last = part.saturating_add(1) == approval.parts;
			// Canonical cut: every part but the last is full.
			ensure!(
				is_last || entries.len() == T::MaxEntriesPerPart::get() as usize,
				Error::<T>::NonCanonicalPart
			);
			let expected = approval
				.part_hashes
				.get(usize::from(part))
				.ok_or(Error::<T>::PartOutOfOrder)?;
			ensure!(
				st::part_hash(&Self::genesis_hash(), window, revision, part, &entries) == *expected,
				Error::<T>::PartHashMismatch
			);

			// No predecessor before the first entry of part 0.
			let mut last = if part == 0 { None } else { Some(progress.last_family.clone()) };
			let mut usd_total = progress.usd_paid;
			let mut part_usd: u128 = 0;
			let mut part_alpha: u128 = 0;
			for entry in entries.iter() {
				ensure!(
					last.as_ref().map_or(true, |prev| entry.family > *prev),
					Error::<T>::FamiliesNotAscending
				);
				ensure!(entry.usd > 0, Error::<T>::ZeroUsd);
				ensure!(
					st::alpha_for(entry.usd, approval.price) == Some(entry.alpha),
					Error::<T>::AlphaMismatch
				);
				usd_total = usd_total
					.checked_add(entry.usd)
					.filter(|total| *total <= approval.usd)
					.ok_or(Error::<T>::TotalsExceeded)?;
				part_usd = part_usd.saturating_add(entry.usd);
				part_alpha = part_alpha.checked_add(entry.alpha).ok_or(Error::<T>::AlphaOverflow)?;
				last = Some(entry.family.clone());
			}
			if is_last {
				ensure!(usd_total == approval.usd, Error::<T>::TotalsMismatch);
			}

			ensure!(
				part_alpha <= Self::settlement_spendable(),
				Error::<T>::InsufficientSettlementFunds
			);
			let today = T::ComputeRevenue::current_window();
			let day = u64::from(today);
			let (counter_day, spent) = ComputeDailyAlpha::<T>::get();
			let spent = if counter_day == day { spent } else { 0 };
			let max_daily = ComputeMaxDailyAlpha::<T>::get();
			let spent = spent
				.checked_add(part_alpha)
				.filter(|s| *s <= max_daily)
				.ok_or(Error::<T>::DailyAlphaCapExceeded)?;

			let settlement = Self::settlement_account();
			for entry in entries.iter() {
				if entry.alpha == 0 {
					continue;
				}
				Self::pay_or_owe(&settlement, &entry.family, entry.alpha.saturated_into())?;
			}

			progress.next_part = part.saturating_add(1);
			if let Some(last) = last {
				progress.last_family = last;
			}
			progress.usd_paid = usd_total;
			progress.alpha_paid = progress.alpha_paid.saturating_add(part_alpha);
			ComputeProgress::<T>::insert((window, revision), &progress);
			ComputeDailyAlpha::<T>::put((day, spent));
			ApprovedUnpaidAlpha::<T>::mutate(|r| *r = r.saturating_sub(part_alpha));
			Self::deposit_event(Event::ComputePartPaid {
				window,
				revision,
				part,
				usd: part_usd,
				alpha: part_alpha,
			});

			if is_last {
				Self::complete_approval(window, revision, today);
			}
			Ok(Pays::No.into())
		}

		/// Claim everything owed to the caller.
		#[pallet::call_index(12)]
		#[pallet::weight(<T as Config>::WeightInfo::claim_compute_owed())]
		pub fn claim_compute_owed(origin: OriginFor<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			Self::ensure_switches()?;
			let amount = ComputeOwed::<T>::get(&who);
			ensure!(!amount.is_zero(), Error::<T>::NothingOwed);
			T::Currency::transfer(
				&Self::owed_account(),
				&who,
				amount,
				ExistenceRequirement::KeepAlive,
			)?;
			ComputeOwed::<T>::remove(&who);
			ComputeOwedTotal::<T>::mutate(|t| *t = t.saturating_sub(amount));
			Self::deposit_event(Event::ComputeOwedClaimed { family: who, alpha: amount });
			Ok(())
		}

		/// Move `amount` from the caller into the settlement account.
		#[pallet::call_index(13)]
		#[pallet::weight(<T as Config>::WeightInfo::fund_compute_settlement())]
		pub fn fund_compute_settlement(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);
			T::Currency::transfer(
				&who,
				&Self::settlement_account(),
				amount,
				ExistenceRequirement::KeepAlive,
			)?;
			Self::deposit_event(Event::ComputeSettlementFunded { who, amount });
			Ok(())
		}

		/// Revoke an approval nothing has been paid from yet.
		///
		/// Origin: a submitter; `backend_sig` by an approver key over
		/// [`st::revoke_digest`]. The `(window, revision)` slot can then be
		/// approved again. Works with the switches off. Feeless on success.
		#[pallet::call_index(14)]
		#[pallet::weight(<T as Config>::WeightInfo::revoke_compute_approval())]
		pub fn revoke_compute_approval(
			origin: OriginFor<T>,
			window: u32,
			revision: u8,
			backend_sig: sr25519::Signature,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
			Self::ensure_submitter(&who)?;
			let approval =
				ComputeApproval::<T>::get((window, revision)).ok_or(Error::<T>::NoApproval)?;
			let progress =
				ComputeProgress::<T>::get((window, revision)).ok_or(Error::<T>::NoApproval)?;
			// In flight and untouched: a completed approval is no longer in
			// flight, a started one has paid a part.
			ensure!(
				progress.next_part == 0
					&& InFlightApproval::<T>::get() == Some((window, revision)),
				Error::<T>::ApprovalAlreadyStarted
			);
			let digest = st::revoke_digest(
				&Self::genesis_hash(),
				window,
				revision,
				&approval.approval_digest,
			);
			Self::ensure_approver_signature(&digest, &backend_sig)?;

			ComputeApproval::<T>::remove((window, revision));
			ComputeProgress::<T>::remove((window, revision));
			if revision == 0 {
				WindowParams::<T>::remove(window);
			}
			let reserved = approval.reserved_alpha();
			ApprovedUnpaidAlpha::<T>::mutate(|r| *r = r.saturating_sub(reserved));
			InFlightApproval::<T>::kill();
			Self::deposit_event(Event::ComputeApprovalRevoked { window, revision });
			Ok(Pays::No.into())
		}

		/// Root schedules a settlement change, applied by anyone once
		/// `RootChangeDelay` has passed. A change of the same kind replaces
		/// the pending one and restarts the delay.
		#[pallet::call_index(15)]
		#[pallet::weight(<T as Config>::WeightInfo::schedule_compute_root_change())]
		pub fn schedule_compute_root_change(
			origin: OriginFor<T>,
			change: st::RootChange<T::AccountId>,
		) -> DispatchResult {
			T::SettlementRootOrigin::ensure_origin(origin)?;
			Self::validate_root_change(&change)?;
			let kind = change.kind();
			let effective = frame_system::Pallet::<T>::block_number()
				.saturating_add(T::RootChangeDelay::get());
			PendingRootChange::<T>::insert(kind, (change, effective));
			Self::deposit_event(Event::RootChangeScheduled { kind, effective });
			Ok(())
		}

		/// Apply a pending settlement change whose delay has passed. Any
		/// signed origin; the change is re-validated against current state.
		#[pallet::call_index(16)]
		#[pallet::weight(<T as Config>::WeightInfo::apply_compute_root_change())]
		pub fn apply_compute_root_change(origin: OriginFor<T>, kind: st::ChangeKind) -> DispatchResult {
			ensure_signed(origin)?;
			let (change, effective) =
				PendingRootChange::<T>::get(kind).ok_or(Error::<T>::NoPendingChange)?;
			ensure!(
				frame_system::Pallet::<T>::block_number() >= effective,
				Error::<T>::ChangeNotDue
			);
			Self::validate_root_change(&change)?;
			PendingRootChange::<T>::remove(kind);
			Self::apply_root_change(change)?;
			Self::deposit_event(Event::RootChangeApplied { kind });
			Ok(())
		}

		/// Root drops a pending settlement change.
		#[pallet::call_index(17)]
		#[pallet::weight(<T as Config>::WeightInfo::cancel_compute_root_change())]
		pub fn cancel_compute_root_change(origin: OriginFor<T>, kind: st::ChangeKind) -> DispatchResult {
			T::SettlementRootOrigin::ensure_origin(origin)?;
			ensure!(PendingRootChange::<T>::contains_key(kind), Error::<T>::NoPendingChange);
			PendingRootChange::<T>::remove(kind);
			Self::deposit_event(Event::RootChangeCancelled { kind });
			Ok(())
		}

		/// Switch settlement off at once, and drop a pending enable. The
		/// admin or root.
		#[pallet::call_index(18)]
		#[pallet::weight(<T as Config>::WeightInfo::disable_compute_settlement())]
		pub fn disable_compute_settlement(origin: OriginFor<T>) -> DispatchResult {
			if let Err(origin) = T::AdminOrigin::try_origin(origin) {
				T::SettlementRootOrigin::ensure_origin(origin)?;
			}
			ComputeSettlementEnabled::<T>::put(false);
			PendingRootChange::<T>::remove(st::ChangeKind::Enable);
			Self::deposit_event(Event::ComputeSettlementEnabledChanged { enabled: false });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Settlement funds (`cmp-settle`). Unreachable from every other
		/// bank path, which only spends [`Self::account_id`].
		pub fn settlement_account() -> T::AccountId {
			T::PalletId::get().into_sub_account_truncating(b"cmp-settle")
		}

		/// Failed settlement transfers (`cmp-owed`), claimable by families.
		pub fn owed_account() -> T::AccountId {
			T::PalletId::get().into_sub_account_truncating(b"cmp-owed")
		}

		/// Settlement balance above its seed (the existential deposit).
		pub fn settlement_spendable() -> u128 {
			T::Currency::free_balance(&Self::settlement_account())
				.saturating_sub(T::Currency::minimum_balance())
				.saturated_into()
		}

		/// The chain's block 0 hash, bound into every settlement digest.
		pub fn genesis_hash() -> H256 {
			let hash = frame_system::Pallet::<T>::block_hash(BlockNumberFor::<T>::zero());
			H256::decode(&mut &hash.encode()[..]).unwrap_or_default()
		}

		/// The all-zero account, `Progress::last_family` before part 0.
		fn zero_account() -> T::AccountId {
			T::AccountId::decode(&mut sp_runtime::traits::TrailingZeroInput::zeroes())
				.expect("an all-zero input decodes any fixed-width account id; qed")
		}

		fn ensure_switches() -> DispatchResult {
			ensure!(ComputeSettlementEnabled::<T>::get(), Error::<T>::ComputeSettlementDisabled);
			ensure!(DistributionEnabled::<T>::get(), Error::<T>::DistributionDisabled);
			Ok(())
		}

		fn ensure_submitter(who: &T::AccountId) -> DispatchResult {
			ensure!(
				ComputeSettlementSubmitters::<T>::get().contains(who),
				Error::<T>::NotComputeSubmitter
			);
			Ok(())
		}

		/// The account an approver key would sign transactions as.
		fn approver_account(key: &sr25519::Public) -> Option<T::AccountId> {
			T::AccountId::decode(&mut &key.0[..]).ok()
		}

		/// Submitters and approvers: non-empty, unique, disjoint, and none
		/// holding another privileged role.
		pub(crate) fn validate_keys(
			submitters: &[T::AccountId],
			approvers: &[sr25519::Public],
		) -> DispatchResult {
			ensure!(!submitters.is_empty() && !approvers.is_empty(), Error::<T>::InvalidSettlementKeys);
			let mut accounts: Vec<T::AccountId> = submitters.to_vec();
			for key in approvers {
				accounts
					.push(Self::approver_account(key).ok_or(Error::<T>::InvalidSettlementKeys)?);
			}
			for (i, a) in accounts.iter().enumerate() {
				ensure!(!accounts[..i].contains(a), Error::<T>::InvalidSettlementKeys);
				ensure!(
					!T::SettlementKeyRoles::has_other_role(a),
					Error::<T>::SettlementKeyHasOtherRole
				);
			}
			Ok(())
		}

		fn ensure_current_keys_valid() -> DispatchResult {
			Self::validate_keys(
				&ComputeSettlementSubmitters::<T>::get(),
				&ComputeSettlementApproverKeys::<T>::get(),
			)
		}

		fn ensure_approver_signature(digest: &H256, sig: &sr25519::Signature) -> DispatchResult {
			let ok = ComputeSettlementApproverKeys::<T>::get()
				.iter()
				.any(|key| sp_io::crypto::sr25519_verify(sig, digest.as_bytes(), key));
			ensure!(ok, Error::<T>::BadApproverSignature);
			Ok(())
		}

		/// `AlphaPrice`, refused when unset or older than `MaxAlphaPriceAge`.
		fn fresh_alpha_price() -> Result<u128, DispatchError> {
			let (price, updated_at) = T::AlphaPrice::alpha_price();
			ensure!(price > 0, Error::<T>::AlphaPriceStale);
			let updated_at = updated_at.ok_or(Error::<T>::AlphaPriceStale)?;
			let age = frame_system::Pallet::<T>::block_number().saturating_sub(updated_at);
			ensure!(age <= T::MaxAlphaPriceAge::get(), Error::<T>::AlphaPriceStale);
			Ok(price)
		}

		/// Order, revision and params rules; returns the params the approval
		/// is checked under (the latest version for revision 0, the pinned
		/// one for later revisions).
		fn check_order_and_params(
			window: u32,
			revision: u8,
			today: u32,
			input: &st::ApprovalInput,
		) -> Result<st::PayoutParams, DispatchError> {
			ensure!(input.formula_version == st::FORMULA_VERSION, Error::<T>::ParamsMismatch);
			let (version, params, formula) = if revision == 0 {
				ensure!(window == NextComputeWindow::<T>::get(), Error::<T>::WindowOutOfOrder);
				let (version, params) =
					ComputePayoutParams::<T>::get().ok_or(Error::<T>::ParamsNotSet)?;
				(version, params, st::FORMULA_VERSION)
			} else {
				ensure!(
					today <= window.saturating_add(T::ComputeRevisionHorizon::get()),
					Error::<T>::RevisionTooLate
				);
				ensure!(
					ComputeRevisionSeq::<T>::contains_key((window, revision - 1)),
					Error::<T>::RevisionOutOfOrder
				);
				let last_day = ComputeLastRevisionDay::<T>::get(window)
					.ok_or(Error::<T>::RevisionOutOfOrder)?;
				ensure!(last_day < today, Error::<T>::RevisionSameDay);
				WindowParams::<T>::get(window).ok_or(Error::<T>::RevisionOutOfOrder)?
			};
			ensure!(
				input.params_version == version
					&& input.formula_version == formula
					&& input.params_hash == st::params_hash(&params),
				Error::<T>::ParamsMismatch
			);
			Ok(params)
		}

		/// Transfer to `family`, or move the amount to the owed account if
		/// that transfer fails. Only the move to the owed account may fail
		/// the call.
		fn pay_or_owe(
			settlement: &T::AccountId,
			family: &T::AccountId,
			amount: BalanceOf<T>,
		) -> DispatchResult {
			let paid = with_storage_layer(|| {
				T::Currency::transfer(settlement, family, amount, ExistenceRequirement::KeepAlive)
			});
			if paid.is_ok() {
				Self::deposit_event(Event::ComputeMinerSettled {
					family: family.clone(),
					alpha: amount,
				});
				return Ok(());
			}
			T::Currency::transfer(
				settlement,
				&Self::owed_account(),
				amount,
				ExistenceRequirement::KeepAlive,
			)
			.map_err(|_| Error::<T>::OwedTransferFailed)?;
			ComputeOwed::<T>::mutate(family, |o| *o = o.saturating_add(amount));
			ComputeOwedTotal::<T>::mutate(|t| *t = t.saturating_add(amount));
			Self::deposit_event(Event::ComputeOwedRecorded { family: family.clone(), alpha: amount });
			Ok(())
		}

		/// Mark `(window, revision)` complete on `today`, add it to the
		/// window's paid totals, release the rounding dust of its reservation;
		/// revision 0 opens the next window.
		fn complete_approval(window: u32, revision: u8, today: u32) {
			let Some(approval) = ComputeApproval::<T>::get((window, revision)) else { return };
			let Some(progress) = ComputeProgress::<T>::get((window, revision)) else { return };
			let dust = approval.reserved_alpha().saturating_sub(progress.alpha_paid);
			ApprovedUnpaidAlpha::<T>::mutate(|r| *r = r.saturating_sub(dust));
			ComputeWindowPaid::<T>::mutate(window, |(usd, alpha)| {
				*usd = usd.saturating_add(progress.usd_paid);
				*alpha = alpha.saturating_add(progress.alpha_paid);
			});
			let seq = NextRevisionSeq::<T>::get();
			ComputeRevisionSeq::<T>::insert((window, revision), seq);
			NextRevisionSeq::<T>::put(seq.saturating_add(1));
			ComputeLastRevisionDay::<T>::insert(window, today);
			InFlightApproval::<T>::kill();
			if revision == 0 {
				NextComputeWindow::<T>::put(window.saturating_add(1));
			}
			Self::deposit_event(Event::ComputeWindowCompleted {
				window,
				revision,
				usd: progress.usd_paid,
				alpha: progress.alpha_paid,
			});
		}

		/// Checks a change can be scheduled or applied against current state.
		fn validate_root_change(change: &st::RootChange<T::AccountId>) -> DispatchResult {
			match change {
				st::RootChange::Keys { submitters, approvers } => {
					Self::validate_keys(submitters, approvers)
				},
				st::RootChange::Params(p) => {
					// `r + ε < 1` is the wash-trading bound (G1).
					ensure!(
						u32::from(p.share_bps) + u32::from(p.tol_bps) < st::BPS
							&& p.share_bps > 0 && p.vcpu_hour_uusd > 0,
						Error::<T>::InvalidParams
					);
					Ok(())
				},
				st::RootChange::MaxDailyAlpha(_) => Ok(()),
				st::RootChange::FirstWindow(_) => {
					ensure!(
						FirstComputeWindow::<T>::get().is_none(),
						Error::<T>::FirstWindowAlreadySet
					);
					Ok(())
				},
				st::RootChange::Enable | st::RootChange::RevenueAccount(_) => Ok(()),
				st::RootChange::Defund(amount) => {
					ensure!(*amount > 0, Error::<T>::ZeroAmount);
					Ok(())
				},
			}
		}

		fn apply_root_change(change: st::RootChange<T::AccountId>) -> DispatchResult {
			match change {
				st::RootChange::Keys { submitters, approvers } => {
					ComputeSettlementSubmitters::<T>::put(submitters);
					ComputeSettlementApproverKeys::<T>::put(approvers);
				},
				st::RootChange::Params(params) => {
					let version =
						ComputePayoutParams::<T>::get().map_or(1, |(v, _)| v.saturating_add(1));
					ComputePayoutParams::<T>::put((version, params));
				},
				st::RootChange::MaxDailyAlpha(max) => ComputeMaxDailyAlpha::<T>::put(max),
				st::RootChange::FirstWindow(window) => {
					FirstComputeWindow::<T>::put(window);
					NextComputeWindow::<T>::put(window);
				},
				st::RootChange::Enable => {
					ComputeSettlementEnabled::<T>::put(true);
					Self::deposit_event(Event::ComputeSettlementEnabledChanged { enabled: true });
				},
				st::RootChange::RevenueAccount(account) => {
					ComputeRevenueAccount::<T>::put(account);
				},
				st::RootChange::Defund(amount) => {
					let to = ComputeRevenueAccount::<T>::get().ok_or(Error::<T>::NoRevenueAccount)?;
					let headroom = Self::settlement_spendable()
						.saturating_sub(ApprovedUnpaidAlpha::<T>::get())
						.saturating_sub(ComputeOwedTotal::<T>::get().saturated_into());
					let amount = amount.min(headroom);
					ensure!(amount > 0, Error::<T>::NothingToDefund);
					let amount: BalanceOf<T> = amount.saturated_into();
					T::Currency::transfer(
						&Self::settlement_account(),
						&to,
						amount,
						ExistenceRequirement::KeepAlive,
					)?;
					Self::deposit_event(Event::ComputeSettlementDefunded { to, amount });
				},
			}
			Ok(())
		}
		/// The bank sovereign account.
		pub fn account_id() -> T::AccountId {
			T::PalletId::get().into_account_truncating()
		}

		/// Record a fee deposit in the bank's ledger and emit the Deposited event.
		/// Used by the transaction fee handler to track all tx fees.
		pub fn record_fee_deposit(amount: BalanceOf<T>) {
			TotalDeposited::<T>::mutate(DepositType::Fees, |total| {
				*total = total.saturating_add(amount);
			});
			Self::deposit_event(Event::Deposited {
				who: Self::account_id(),
				amount,
				deposit_type: DepositType::Fees,
			});
		}

		/// Funds `request_payment` could release right now (free balance minus
		/// the existential deposit the bank always keeps).
		pub fn available_for_payout() -> BalanceOf<T> {
			payment_math::available(
				Tokens::new(T::Currency::free_balance(&Self::account_id()).saturated_into()),
				Tokens::new(T::Currency::minimum_balance().saturated_into()),
			)
			.get()
			.saturated_into()
		}

		/// Emission deposited but not yet distributed to storage miners.
		///
		/// Other bank consumers (arion settlement, referral commissions) subtract
		/// this from their spendable headroom — the compartment is reserved for
		/// `pay_storage_miners`.
		pub fn emission_available() -> BalanceOf<T> {
			TotalDeposited::<T>::get(DepositType::Emission)
				.saturating_sub(EmissionPaidOut::<T>::get())
		}

		/// What `request_payment` may actually release: the free balance above
		/// ED, minus BOTH emission compartments.
		///
		/// The compartments are reserved for `pay_storage_miners` /
		/// `pay_compute_miners`, and that reservation has to be enforced here
		/// rather than left to each caller. Callers that subtract it themselves
		/// (the arion payout source, the referral commission path) clamp their
		/// own amount first, so this second ceiling is a no-op for them; the
		/// callers that don't — the sudo-refund retry and the chargeback refund
		/// — were able to drain the bank down to ED while the compartment
		/// ledger still showed a full balance, leaving `pay_compute_miners` to
		/// fail with `InsufficientBankBalance` against emission that was
		/// nominally reserved for it.
		///
		/// This bounds only what THIS pallet knows it owes. Runtime-level
		/// reservations (undistributed marketplace backing, pending sudo
		/// refunds) are still the caller's to subtract.
		pub fn unreserved_for_payout() -> BalanceOf<T> {
			Self::available_for_payout()
				.saturating_sub(Self::emission_available())
				.saturating_sub(Self::compute_emission_available())
		}

		/// Compute emission deposited but not yet distributed to compute
		/// miners.
		///
		/// Like [`Self::emission_available`], other bank consumers must
		/// subtract this from their spendable headroom — the compartment is
		/// reserved for `pay_compute_miners`.
		pub fn compute_emission_available() -> BalanceOf<T> {
			TotalDeposited::<T>::get(DepositType::ComputeEmission)
				.saturating_sub(ComputeEmissionPaidOut::<T>::get())
		}

		/// Transfer `amount` from `who` into the bank and record it. Shared by
		/// the `deposit` extrinsic and other pallets (e.g. the marketplace
		/// routing deposit alpha backing to the bank).
		pub fn deposit_from(
			who: &T::AccountId,
			amount: BalanceOf<T>,
			deposit_type: DepositType,
		) -> DispatchResult {
			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);
			T::Currency::transfer(
				who,
				&Self::account_id(),
				amount,
				ExistenceRequirement::KeepAlive,
			)?;
			TotalDeposited::<T>::mutate(deposit_type, |t| *t = t.saturating_add(amount));
			Self::deposit_event(Event::Deposited { who: who.clone(), amount, deposit_type });
			Ok(())
		}

		/// Free balance held by the bank.
		pub fn balance() -> BalanceOf<T> {
			T::Currency::free_balance(&Self::account_id())
		}

		/// Internal payment API — deliberately NOT an extrinsic.
		///
		/// Releases up to `amount` from the bank to `dest` on behalf of a
		/// whitelisted `requester` (typically another pallet's sovereign
		/// account). Never overdraws: pays `min(amount, free - ED)` and returns
		/// the amount actually paid so the caller can account for the shortfall.
		///
		/// Fails with [`Error::DistributionDisabled`] while the global
		/// [`DistributionEnabled`] switch is off — checked before the whitelist,
		/// so no funds move and no ledger entry is written regardless of the
		/// requester's standing.
		pub fn request_payment(
			requester: &T::AccountId,
			dest: &T::AccountId,
			amount: BalanceOf<T>,
		) -> Result<BalanceOf<T>, DispatchError> {
			ensure!(DistributionEnabled::<T>::get(), Error::<T>::DistributionDisabled);
			ensure!(
				WhitelistedRequesters::<T>::contains_key(requester),
				Error::<T>::RequesterNotWhitelisted
			);
			if amount.is_zero() {
				return Ok(Zero::zero());
			}
			let bank = Self::account_id();
			// Bound how much a single call can move. This limits the blast
			// radius of one runaway request; it does not bound total spend,
			// because nothing here consults what the requester already took.
			let capped_amount = match RequesterWithdrawalCap::<T>::get(requester) {
				Some(cap) => amount.min(cap),
				None => amount,
			};
			// Clamped against the UNRESERVED balance, not the raw free balance:
			// a requester must not be able to spend emission the bank is
			// holding for the two miner payouts.
			let paid: BalanceOf<T> = payment_math::payable(
				Tokens::new(capped_amount.saturated_into()),
				Tokens::new(Self::unreserved_for_payout().saturated_into()),
			)
			.get()
			.saturated_into();
			if !paid.is_zero() {
				T::Currency::transfer(&bank, dest, paid, ExistenceRequirement::KeepAlive)?;
				TotalPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
				TotalPaidByRequester::<T>::mutate(requester, |t| *t = t.saturating_add(paid));
				Self::deposit_event(Event::PaymentReleased {
					requester: requester.clone(),
					dest: dest.clone(),
					requested: amount,
					paid,
				});
			}
			Ok(paid)
		}
	}
}
