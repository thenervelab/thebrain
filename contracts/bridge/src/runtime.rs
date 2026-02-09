#![allow(clippy::cast_possible_truncation)]

use ink::prelude::boxed::Box;
use ink::primitives::AccountId;
use ink::scale::{Compact, CompactAs, Error as CodecError};
use sp_runtime::MultiAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct AlphaCurrency(u64);

impl AlphaCurrency {
	pub fn as_u64(&self) -> u64 {
		self.0
	}
}

impl From<u64> for AlphaCurrency {
	fn from(value: u64) -> Self {
		AlphaCurrency(value)
	}
}

impl CompactAs for AlphaCurrency {
	type As = u64;

	fn encode_as(&self) -> &Self::As {
		&self.0
	}

	fn decode_from(v: Self::As) -> Result<Self, CodecError> {
		Ok(Self(v))
	}
}

impl From<Compact<AlphaCurrency>> for AlphaCurrency {
	fn from(c: Compact<AlphaCurrency>) -> Self {
		c.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub struct NetUid(u16);

impl NetUid {
	pub fn as_u16(&self) -> u16 {
		self.0
	}
}

impl From<u16> for NetUid {
	fn from(value: u16) -> Self {
		NetUid(value)
	}
}

impl CompactAs for NetUid {
	type As = u16;

	fn encode_as(&self) -> &Self::As {
		&self.0
	}

	fn decode_from(v: Self::As) -> Result<Self, CodecError> {
		Ok(Self(v))
	}
}

impl From<Compact<NetUid>> for NetUid {
	fn from(c: Compact<NetUid>) -> Self {
		c.0
	}
}

#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum ProxyType {
	Any,
	Owner,
	NonCritical,
	NonTransfer,
	Senate,
	NonFungibile,
	Triumvirate,
	Governance,
	Staking,
	Registration,
	Transfer,
	SmallTransfer,
	RootWeights,
	ChildKeys,
	SudoUncheckedSetCode,
	SwapHotkey,
	SubnetLeaseBeneficiary,
}

#[ink::scale_derive(Encode)]
pub enum RuntimeCall {
	#[codec(index = 7)]
	SubtensorModule(SubtensorCall),
	#[codec(index = 16)]
	Proxy(ProxyCall),
}

#[ink::scale_derive(Encode)]
pub enum ProxyCall {
	#[codec(index = 0)]
	Proxy {
		real: MultiAddress<AccountId, ()>,
		force_proxy_type: Option<ProxyType>,
		call: Box<RuntimeCall>,
	},
}

#[ink::scale_derive(Encode)]
pub enum SubtensorCall {
	#[codec(index = 86)]
	TransferStake {
		destination_coldkey: AccountId,
		hotkey: AccountId,
		origin_netuid: NetUid,
		destination_netuid: NetUid,
		alpha_amount: AlphaCurrency,
	},
}