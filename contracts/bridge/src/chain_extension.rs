use crate::runtime::{AlphaCurrency, NetUid};
use ink::primitives::AccountId;
use ink::scale::Compact;

#[derive(PartialEq, Eq, Clone, Debug)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct StakeInfo {
	hotkey: AccountId,
	coldkey: AccountId,
	netuid: Compact<NetUid>,
	stake: Compact<AlphaCurrency>,
	locked: Compact<u64>,
	emission: Compact<AlphaCurrency>,
	tao_emission: Compact<u64>,
	drain: Compact<u64>,
	is_registered: bool,
}

impl StakeInfo {
	pub fn stake_amount(&self) -> u64 {
		let alpha_currency = self.stake.0;
		alpha_currency.as_u64()
	}

	#[cfg(test)]
	pub(crate) fn new_for_tests(
		hotkey: AccountId,
		coldkey: AccountId,
		netuid: NetUid,
		stake: AlphaCurrency,
	) -> Self {
		Self {
			hotkey,
			coldkey,
			netuid: Compact(netuid),
			stake: Compact(stake),
			locked: Compact(0),
			emission: Compact(AlphaCurrency::from(0)),
			tao_emission: Compact(0),
			drain: Compact(0),
			is_registered: true,
		}
	}
}

/// ABI-coupled to the Subtensor chain-extension `StakeAvailability` type.
/// The runtime returns plain `u64` fields here, not compact-encoded balances.
/// `total` and `locked` are retained to keep the SCALE layout pinned to the
/// runtime type; the contract consumes `total` as an ABI sanity check.
#[derive(PartialEq, Eq, Copy, Clone, Debug)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct StakeAvailability {
	netuid: NetUid,
	total: u64,
	locked: u64,
	available: u64,
}

impl StakeAvailability {
	pub fn netuid(&self) -> NetUid {
		self.netuid
	}

	pub fn available_amount(&self) -> u64 {
		self.available
	}

	pub fn total_amount(&self) -> u64 {
		self.total
	}

	#[cfg(test)]
	pub(crate) fn new_for_tests(netuid: NetUid, total: u64, locked: u64, available: u64) -> Self {
		Self { netuid, total, locked, available }
	}
}

#[allow(clippy::cast_possible_truncation)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum SubtensorError {
	UnknownStatusCode,
	RuntimeError = 1,
	NotEnoughBalanceToStake = 2,
	NonAssociatedColdKey = 3,
	BalanceWithdrawalError = 4,
	NotRegistered = 5,
	NotEnoughStakeToWithdraw = 6,
	TxRateLimitExceeded = 7,
	SlippageTooHigh = 8,
	SubnetNotExists = 9,
	HotKeyNotRegisteredInSubNet = 10,
	SameAutoStakeHotkeyAlreadySet = 11,
	InsufficientBalance = 12,
	AmountTooLow = 13,
	InsufficientLiquidity = 14,
	SameNetuid = 15,
	InvalidScaleEncoding,
}

impl From<ink::scale::Error> for SubtensorError {
	fn from(_: ink::scale::Error) -> Self {
		SubtensorError::InvalidScaleEncoding
	}
}

impl ink::env::chain_extension::FromStatusCode for SubtensorError {
	fn from_status_code(status_code: u32) -> Result<(), Self> {
		match status_code {
			0 => Ok(()),
			1 => Err(SubtensorError::RuntimeError),
			2 => Err(SubtensorError::NotEnoughBalanceToStake),
			3 => Err(SubtensorError::NonAssociatedColdKey),
			4 => Err(SubtensorError::BalanceWithdrawalError),
			5 => Err(SubtensorError::NotRegistered),
			6 => Err(SubtensorError::NotEnoughStakeToWithdraw),
			7 => Err(SubtensorError::TxRateLimitExceeded),
			8 => Err(SubtensorError::SlippageTooHigh),
			9 => Err(SubtensorError::SubnetNotExists),
			10 => Err(SubtensorError::HotKeyNotRegisteredInSubNet),
			11 => Err(SubtensorError::SameAutoStakeHotkeyAlreadySet),
			12 => Err(SubtensorError::InsufficientBalance),
			13 => Err(SubtensorError::AmountTooLow),
			14 => Err(SubtensorError::InsufficientLiquidity),
			15 => Err(SubtensorError::SameNetuid),
			_ => Err(SubtensorError::UnknownStatusCode),
		}
	}
}

#[ink::chain_extension(extension = 0)]
pub trait SubtensorExtension {
	type ErrorCode = SubtensorError;

	#[ink(function = 0)]
	fn get_stake_info(
		hotkey: AccountId,
		coldkey: AccountId,
		netuid: NetUid,
	) -> Result<Option<StakeInfo>, SubtensorError>;

	#[ink(function = 5)]
	fn move_stake(
		origin_hotkey: AccountId,
		destination_hotkey: AccountId,
		origin_netuid: NetUid,
		destination_netuid: NetUid,
		alpha_amount: AlphaCurrency,
	) -> Result<(), SubtensorError>;

	#[ink(function = 6)]
	fn transfer_stake(
		destination_coldkey: AccountId,
		hotkey: AccountId,
		origin_netuid: NetUid,
		destination_netuid: NetUid,
		alpha_amount: AlphaCurrency,
	) -> Result<(), SubtensorError>;

	#[ink(function = 36)]
	fn get_stake_availability(
		coldkey: AccountId,
		netuid: NetUid,
	) -> Result<StakeAvailability, SubtensorError>;
}
