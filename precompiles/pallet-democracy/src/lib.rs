// This file is part of The Brain.
// Copyright (C) 2022-2024 The Nerve Lab
//
// This file is part of pallet-evm-precompile-democracy package, originally developed by Purestake
// Inc. Pallet-evm-precompile-democracy package used in Hippius Network in terms of GPLv3.

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Precompile to interact with pallet democracy through an evm precompile.

#![cfg_attr(not(feature = "std"), no_std)]

use fp_evm::PrecompileHandle;
use frame_support::{
	dispatch::{GetDispatchInfo, PostDispatchInfo},
	traits::{Bounded, ConstU32, Currency, QueryPreimage},
};
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_democracy::{
	AccountVote, Call as DemocracyCall, Conviction, ReferendumInfo, Vote, VoteThreshold,
};
use pallet_evm::AddressMapping;
use pallet_preimage::Call as PreimageCall;
use precompile_utils::prelude::*;
use sp_core::{Get, H160, H256, U256};
use sp_runtime::traits::{Dispatchable, Hash, StaticLookup};
use sp_std::{
	convert::{TryFrom, TryInto},
	fmt::Debug,
	marker::PhantomData,
	vec::Vec,
};

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

type BalanceOf<Runtime> = <<Runtime as pallet_democracy::Config>::Currency as Currency<
	<Runtime as frame_system::Config>::AccountId,
>>::Balance;

pub const ENCODED_PROPOSAL_SIZE_LIMIT: u32 = 2u32.pow(16);
type GetEncodedProposalSizeLimit = ConstU32<ENCODED_PROPOSAL_SIZE_LIMIT>;

/// Solidity selector of the Proposed log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_PROPOSED: [u8; 32] = keccak256!("Proposed(uint32,uint256)");

/// Solidity selector of the Seconded log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_SECONDED: [u8; 32] = keccak256!("Seconded(uint32,address)");

/// Solidity selector of the StandardVote log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_STANDARD_VOTE: [u8; 32] =
	keccak256!("StandardVote(uint32,address,bool,uint256,uint8)");

/// Solidity selector of the Delegated log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_DELEGATED: [u8; 32] = keccak256!("Delegated(address,address)");

/// Solidity selector of the Undelegated log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_UNDELEGATED: [u8; 32] = keccak256!("Undelegated(address)");

/// A precompile to wrap the functionality from pallet democracy.
///
/// Grants evm-based DAOs the right to vote making them first-class citizens.
///
/// For an example of a political party that operates as a DAO, see PoliticalPartyDao.sol
pub struct DemocracyPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> DemocracyPrecompile<Runtime>
where
	Runtime: pallet_democracy::Config
		+ pallet_evm::Config
		+ frame_system::Config
		+ pallet_preimage::Config,
	U256: From<BalanceOf<Runtime>>,
	BalanceOf<Runtime>: TryFrom<U256> + TryInto<u128> + Into<U256> + Debug + solidity::Codec,
	Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
	<Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
	Runtime::RuntimeCall: From<DemocracyCall<Runtime>>,
	Runtime::RuntimeCall: From<PreimageCall<Runtime>>,
	Runtime::Hash: From<H256> + Into<H256>,
	BlockNumberFor<Runtime>: Into<U256>,
{
	// The accessors are first. They directly return their result.
	#[precompile::public("publicPropCount()")]
	#[precompile::public("public_prop_count()")]
	#[precompile::view]
	fn public_prop_count(handle: &mut impl PrecompileHandle) -> EvmResult<U256> {
		// Fetch data from pallet
		// storage item: PublicPropCount
		// max encoded len: u32(4)
		handle.record_db_read::<Runtime>(4)?;
		let prop_count = pallet_democracy::PublicPropCount::<Runtime>::get();
		log::trace!(target: "democracy-precompile", "Prop count from pallet is {:?}", prop_count);

		Ok(prop_count.into())
	}

	#[precompile::public("depositOf(uint256)")]
	#[precompile::public("deposit_of(uint256)")]
	#[precompile::view]
	fn deposit_of(
		handle: &mut impl PrecompileHandle,
		prop_index: Convert<U256, u32>,
	) -> EvmResult<U256> {
		let prop_index = prop_index.converted();

		// Fetch data from pallet
		// storage item: DepositOf
		// max encoded len: Twox64(8) + PropIndex u32(4) +
		// BalanceOf(16) + BoundedVec<AccountId, MaxDeposits> (20 * MaxDeposits)
		handle.record_db_read::<Runtime>((20 * <Runtime::MaxDeposits>::get() as usize) + 28)?;
		let deposit = pallet_democracy::DepositOf::<Runtime>::get(prop_index)
			.ok_or_else(|| revert("No such proposal in pallet democracy"))?
			.1;

		log::trace!(
			target: "democracy-precompile",
			"Deposit of proposal {:?} is {:?}", prop_index, deposit
		);

		Ok(deposit.into())
	}

	#[precompile::public("lowestUnbaked()")]
	#[precompile::public("lowest_unbaked()")]
	#[precompile::view]
	fn lowest_unbaked(handle: &mut impl PrecompileHandle) -> EvmResult<U256> {
		// Fetch data from pallet
		// storage item: LowestUnbaked
		// max encoded len: ReferendumIndex u32(4)
		handle.record_db_read::<Runtime>(4)?;
		let lowest_unbaked = pallet_democracy::LowestUnbaked::<Runtime>::get();
		log::trace!(
			target: "democracy-precompile",
			"lowest unbaked referendum is {:?}", lowest_unbaked
		);

		Ok(lowest_unbaked.into())
	}

	#[precompile::public("ongoingReferendumInfo(uint32)")]
	#[precompile::view]
	fn ongoing_referendum_info(
		handle: &mut impl PrecompileHandle,
		ref_index: u32,
	) -> EvmResult<(U256, H256, u8, U256, U256, U256, U256)> {
		// storage item: ReferendumInfoOf
		// max encoded len: Twox64(8) + ReferendumIndex u32(4) +
		// ReferendumInfo (enum(1) + end(4) + proposal(128) + threshold(1) + delay(4) + tally(3*16))
		handle.record_db_read::<Runtime>(186)?;
		let ref_status = match pallet_democracy::ReferendumInfoOf::<Runtime>::get(ref_index) {
			Some(ReferendumInfo::Ongoing(ref_status)) => ref_status,
			Some(ReferendumInfo::Finished { .. }) => Err(revert("Referendum is finished"))?,
			None => Err(revert("Unknown referendum"))?,
		};

		let threshold_u8: u8 = match ref_status.threshold {
			VoteThreshold::SuperMajorityApprove => 0,
			VoteThreshold::SuperMajorityAgainst => 1,
			VoteThreshold::SimpleMajority => 2,
		};

		Ok((
			ref_status.end.into(),
			ref_status.proposal.hash().into(),
			threshold_u8,
			ref_status.delay.into(),
			ref_status.tally.ayes.into(),
			ref_status.tally.nays.into(),
			ref_status.tally.turnout.into(),
		))
	}

	#[precompile::public("finishedReferendumInfo(uint32)")]
	#[precompile::view]
	fn finished_referendum_info(
		handle: &mut impl PrecompileHandle,
		ref_index: u32,
	) -> EvmResult<(bool, U256)> {
		// storage item: ReferendumInfoOf
		// max encoded len: Twox64(8) + ReferendumIndex u32(4) +
		// ReferendumInfo (enum(1) + end(4) + proposal(128) + threshold(1) + delay(4) + tally(3*16))
		handle.record_db_read::<Runtime>(186)?;
		let (approved, end) = match pallet_democracy::ReferendumInfoOf::<Runtime>::get(ref_index) {
			Some(ReferendumInfo::Ongoing(_)) => Err(revert("Referendum is ongoing"))?,
			Some(ReferendumInfo::Finished { approved, end }) => (approved, end),
			None => Err(revert("Unknown referendum"))?,
		};

		Ok((approved, end.into()))
	}

	// The dispatchable wrappers are next. They dispatch a Substrate inner Call.
	#[precompile::public("propose(bytes32,uint256)")]
	fn propose(handle: &mut impl PrecompileHandle, proposal_hash: H256, value: U256) -> EvmResult {
		handle.record_log_costs_manual(2, 32)?;

		// Fetch data from pallet
		// storage item: PublicPropCount
		// max encoded len: PropIndex u32(4)
		handle.record_db_read::<Runtime>(4)?;
		let prop_count = pallet_democracy::PublicPropCount::<Runtime>::get();

		let value = Self::u256_to_amount(value).in_field("value")?;

		log::trace!(
			target: "democracy-precompile",
			"Proposing with hash {:?}, and amount {:?}", proposal_hash, value
		);

		// This forces it to have the proposal in pre-images.
		// Storage item: StatusFor
		// max encoded len: Hash(32) + RequestStatus(enum(1) + deposit(1+20+16) + count(4) + len(5))
		handle.record_db_read::<Runtime>(79)?;
		let len = <Runtime as pallet_democracy::Config>::Preimages::len(&proposal_hash.into())
			.ok_or({
				RevertReason::custom("Failure in preimage fetch").in_field("proposal_hash")
			})?;

		let bounded = Bounded::<
			pallet_democracy::CallOf<Runtime>,
			<Runtime as frame_system::Config>::Hashing,
		>::Lookup {
			hash: proposal_hash.into(),
			len,
		};

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::propose { proposal: bounded, value };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		log2(
			handle.context().address,
			SELECTOR_LOG_PROPOSED,
			H256::from_low_u64_be(prop_count as u64), // proposal index,
			solidity::encode_event_data(U256::from(value)),
		)
		.record(handle)?;

		Ok(())
	}

	#[precompile::public("second(uint256,uint256)")]
	fn second(
		handle: &mut impl PrecompileHandle,
		prop_index: Convert<U256, u32>,
		seconds_upper_bound: Convert<U256, u32>,
	) -> EvmResult {
		handle.record_log_costs_manual(2, 32)?;
		let prop_index = prop_index.converted();
		let seconds_upper_bound = seconds_upper_bound.converted();

		log::trace!(
			target: "democracy-precompile",
			"Seconding proposal {:?}, with bound {:?}", prop_index, seconds_upper_bound
		);

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::second { proposal: prop_index };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		log2(
			handle.context().address,
			SELECTOR_LOG_SECONDED,
			H256::from_low_u64_be(prop_index as u64), // proposal index,
			solidity::encode_event_data(Address(handle.context().caller)),
		)
		.record(handle)?;

		Ok(())
	}

	#[precompile::public("standardVote(uint256,bool,uint256,uint256)")]
	#[precompile::public("standard_vote(uint256,bool,uint256,uint256)")]
	fn standard_vote(
		handle: &mut impl PrecompileHandle,
		ref_index: Convert<U256, u32>,
		aye: bool,
		vote_amount: U256,
		conviction: Convert<U256, u8>,
	) -> EvmResult {
		handle.record_log_costs_manual(2, 32 * 4)?;
		let ref_index = ref_index.converted();
		let vote_amount_balance = Self::u256_to_amount(vote_amount).in_field("voteAmount")?;

		let conviction_enum: Conviction = conviction.converted().try_into().map_err(|_| {
			RevertReason::custom("Must be an integer between 0 and 6 included")
				.in_field("conviction")
		})?;

		let vote = AccountVote::Standard {
			vote: Vote { aye, conviction: conviction_enum },
			balance: vote_amount_balance,
		};

		log::trace!(target: "democracy-precompile",
			"Voting {:?} on referendum #{:?}, with conviction {:?}",
			aye, ref_index, conviction_enum
		);

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::vote { ref_index, vote };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		log2(
			handle.context().address,
			SELECTOR_LOG_STANDARD_VOTE,
			H256::from_low_u64_be(ref_index as u64), // referendum index,
			solidity::encode_event_data((
				Address(handle.context().caller),
				aye,
				vote_amount,
				conviction.converted(),
			)),
		)
		.record(handle)?;

		Ok(())
	}

	#[precompile::public("removeVote(uint256)")]
	#[precompile::public("remove_vote(uint256)")]
	fn remove_vote(handle: &mut impl PrecompileHandle, ref_index: Convert<U256, u32>) -> EvmResult {
		let ref_index: u32 = ref_index.converted();

		log::trace!(
			target: "democracy-precompile",
			"Removing vote from referendum {:?}",
			ref_index
		);

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::remove_vote { index: ref_index };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	#[precompile::public("delegate(address,uint256,uint256)")]
	fn delegate(
		handle: &mut impl PrecompileHandle,
		representative: Address,
		conviction: Convert<U256, u8>,
		amount: U256,
	) -> EvmResult {
		handle.record_log_costs_manual(2, 32)?;
		let amount = Self::u256_to_amount(amount).in_field("amount")?;

		let conviction: Conviction = conviction.converted().try_into().map_err(|_| {
			RevertReason::custom("Must be an integer between 0 and 6 included")
				.in_field("conviction")
		})?;

		log::trace!(target: "democracy-precompile",
			"Delegating vote to {representative:?} with balance {amount:?} and conviction {conviction:?}",
		);

		let to = Runtime::AddressMapping::into_account_id(representative.into());
		let to: <Runtime::Lookup as StaticLookup>::Source = Runtime::Lookup::unlookup(to);
		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::delegate { to, conviction, balance: amount };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		log2(
			handle.context().address,
			SELECTOR_LOG_DELEGATED,
			handle.context().caller,
			solidity::encode_event_data(representative),
		)
		.record(handle)?;

		Ok(())
	}

	#[precompile::public("unDelegate()")]
	#[precompile::public("un_delegate()")]
	fn un_delegate(handle: &mut impl PrecompileHandle) -> EvmResult {
		handle.record_log_costs_manual(2, 0)?;
		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::undelegate {};

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		log2(handle.context().address, SELECTOR_LOG_UNDELEGATED, handle.context().caller, [])
			.record(handle)?;

		Ok(())
	}

	#[precompile::public("unlock(address)")]
	fn unlock(handle: &mut impl PrecompileHandle, target: Address) -> EvmResult {
		let target: H160 = target.into();
		let target = Runtime::AddressMapping::into_account_id(target);
		let target: <Runtime::Lookup as StaticLookup>::Source = Runtime::Lookup::unlookup(target);

		log::trace!(
			target: "democracy-precompile",
			"Unlocking democracy tokens for {:?}", target
		);

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = DemocracyCall::<Runtime>::unlock { target };

		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	#[precompile::public("notePreimage(bytes)")]
	#[precompile::public("note_preimage(bytes)")]
	fn note_preimage(
		handle: &mut impl PrecompileHandle,
		encoded_proposal: BoundedBytes<GetEncodedProposalSizeLimit>,
	) -> EvmResult {
		let encoded_proposal: Vec<u8> = encoded_proposal.into();

		log::trace!(
			target: "democracy-precompile",
			"Noting preimage {:?}", encoded_proposal
		);

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = PreimageCall::<Runtime>::note_preimage { bytes: encoded_proposal };
		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	#[precompile::public("noteImminentPreimage(bytes)")]
	#[precompile::public("note_imminent_preimage(bytes)")]
	fn note_imminent_preimage(
		handle: &mut impl PrecompileHandle,
		encoded_proposal: BoundedBytes<GetEncodedProposalSizeLimit>,
	) -> EvmResult {
		let encoded_proposal: Vec<u8> = encoded_proposal.into();

		log::trace!(
			target: "democracy-precompile",
			"Noting imminent preimage {:?}", encoded_proposal
		);

		// To mimic imminent preimage behavior, we need to check whether the preimage
		// has been requested
		let proposal_hash = <Runtime as frame_system::Config>::Hashing::hash(&encoded_proposal);
		// is_requested implies db read
		// Storage item: StatusFor
		// max encoded len: Hash(32) + RequestStatus(enum(1) + deposit(1+20+16) + count(4) + len(5))
		handle.record_db_read::<Runtime>(79)?;
		if !<<Runtime as pallet_democracy::Config>::Preimages as QueryPreimage>::is_requested(
			&proposal_hash,
		) {
			return Err(revert("not imminent preimage (preimage not requested)"));
		};

		let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let call = PreimageCall::<Runtime>::note_preimage { bytes: encoded_proposal };
		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	fn u256_to_amount(value: U256) -> MayRevert<BalanceOf<Runtime>> {
		value
			.try_into()
			.map_err(|_| RevertReason::value_is_too_large("balance type").into())
	}
}
