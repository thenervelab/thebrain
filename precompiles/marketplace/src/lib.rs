// #![cfg_attr(not(feature = "std"), no_std)]
// use fp_evm::PrecompileHandle;
// use frame_support::{
// 	dispatch::{GetDispatchInfo, PostDispatchInfo},
// };
// use pallet_marketplace::Call as MarketplaceCall;
// // use pallet_marketplace::Pallet as MarketplacePallet;
// // use precompile_utils::prelude::*;
// use sp_core::H256;
// use sp_runtime::traits::Dispatchable;
// use sp_std::marker::PhantomData;
// use frame_system::pallet_prelude::BlockNumberFor;
// // use sp_std::vec::Vec;
// use pallet_evm::AddressMapping;
// // use sp_core::crypto::Ss58Codec;
// use sp_runtime::traits::ConstU32;
// // use pallet_marketplace::PermissionLevel;
// // use {
// // 	sp_runtime::traits::StaticLookup,
// // 	frame_support::traits::Get,
// // };
// // use hex;

// #[cfg(test)]
// mod mock;
// #[cfg(test)]
// mod tests;

// /// A precompile to wrap the functionality from pallet author mapping.
// pub struct MarketPlacePrecompile<Runtime>(PhantomData<Runtime>);

// #[precompile_utils::precompile]
// #[precompile::test_concrete_types(mock::Runtime)]
// impl<Runtime> MarketPlacePrecompile<Runtime>
// where
// 	Runtime: pallet_marketplace::Config + pallet_evm::Config + frame_system::Config,
// 	Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
// 	<Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
// 	Runtime::RuntimeCall: From<MarketplaceCall<Runtime>>,
// 	Runtime::Hash: From<H256>,
// 	BlockNumberFor<Runtime>: Into<u64>,
// 	<Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
// {
// 	// // get all the files pinned by a node
// 	// #[precompile::public("getPointBalance(address)")]
// 	// #[precompile::public("get_point_balance(address)")]
// 	// #[precompile::view]
// 	// fn get_point_balance(handle: &mut impl PrecompileHandle, who: Address) -> EvmResult<u128> {
// 	// 	handle.record_db_read::<Runtime>(68)?;
// 	// 	let to_address: H160 = who.into();
// 	// 	let to_account = Runtime::AddressMapping::into_account_id(to_address);
// 	// 	let points = MarketplacePallet::<Runtime>::get_point_balance(&to_account);

// 	// 	Ok(points)
// 	// }

// 	// #[precompile::public("getUserSubscription(address,uint32)")]
// 	// #[precompile::public("get_user_subscription(address,uint32)")]
// 	// fn get_user_subscription(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	user: Address,
// 	// 	subscription_id: u32,
// 	// ) -> EvmResult<(
// 	// 	u32,     // id
// 	// 	u8,      // tier
// 	// 	u32,     // storage_used
// 	// 	u32,     // bandwidth_used
// 	// 	u32,     // requests_used
// 	// 	u64,     // expires_at
// 	// 	bool,    // auto_renewal
// 	// 	bool,    // active
// 	// )> {
// 	// 	handle.record_db_read::<Runtime>(100)?;

// 	// 	// Convert address to AccountId
// 	// 	let user_address: H160 = user.into();
// 	// 	let user_account = Runtime::AddressMapping::into_account_id(user_address);


// 	// 	// Get subscription info
// 	// 	let subscription = MarketplacePallet::<Runtime>::get_user_subscription(user_account, subscription_id)
// 	// 		.ok_or_else(|| revert("Subscription not found"))?;


// 	// 	Ok((
// 	// 		subscription.id,
// 	// 		subscription.package.tier as u8,
// 	// 		subscription.storage_used,
// 	// 		subscription.bandwidth_used,
// 	// 		subscription.requests_used,
// 	// 		subscription.expires_at.into(),
// 	// 		subscription.auto_renewal,
// 	// 		subscription.active,
// 	// 	))
// 	// }

// 	// #[precompile::public("getActiveSubscriptions(address)")]
// 	// #[precompile::public("get_active_subscriptions(address)")]
// 	// fn get_active_subscriptions(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	user: Address,
// 	// ) -> EvmResult<Vec<u32>> {
// 	// 	handle.record_db_read::<Runtime>(100)?;

// 	// 	// Convert address to AccountId
// 	// 	let user_address: H160 = user.into();
// 	// 	let user_account = Runtime::AddressMapping::into_account_id(user_address);

// 	// 	// Get active subscriptions
// 	// 	let active_subs = MarketplacePallet::<Runtime>::get_all_active_subscription_ids(&user_account);

// 	// 	Ok(active_subs)
// 	// }

// 	// #[precompile::public("getPackage(uint8)")]
// 	// #[precompile::public("get_package(uint8)")]
// 	// fn get_package(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	tier: u8,
// 	// ) -> EvmResult<(u8, u32, u32, u32, u32, u32)> {
// 	// 	handle.record_db_read::<Runtime>(100)?;
		
// 	// 	// Get caller's EVM address
// 	// 	let caller = handle.context().caller;
// 	// 	// Convert to Substrate address
// 	// 	let substrate_account = Runtime::AddressMapping::into_account_id(caller);
		

// 	// 	log::info!(
// 	// 		target: "marketplace-precompile",
// 	// 		"Substrate Account Raw: {:?}, SS58: {}",
// 	// 		substrate_account,
// 	// 		AccountId32::new(substrate_account.encode().try_into().unwrap_or_default()).to_ss58check()
// 	// 	);

// 	// 	// Convert u8 to PackageTier
// 	// 	let package_tier = match tier {
// 	// 		0 => PackageTier::Starter,
// 	// 		1 => PackageTier::Quantum,
// 	// 		2 => PackageTier::Pinnacle,
// 	// 		_ => return Err(revert("Invalid package tier"))
// 	// 	};

// 	// 	// Get package details
// 	// 	let package = MarketplacePallet::<Runtime>::get_package(package_tier)
// 	// 		.ok_or_else(|| revert("Package not found"))?;

// 	// 	Ok((
// 	// 		package.tier as u8,        // tier
// 	// 		package.storage_gb,        // storage in GB
// 	// 		package.bandwidth_gb,      // bandwidth in GB
// 	// 		package.requests_limit,    // request limit
// 	// 		package.price_native,      // price in points
// 	// 		package.duration_months,   // duration in months
// 	// 	))
// 	// }

// 	// // #[precompile::public("getSubscriptionStorageRequests(uint256)")]
// 	// // #[precompile::public("get_subscription_storage_requests(uint256)")]
// 	// // #[precompile::view]
// 	// // fn get_subscription_storage_requests(
// 	// // 	handle: &mut impl PrecompileHandle,
// 	// // 	subscription_id: u32,
// 	// // ) -> EvmResult<Vec<(u32, u32, Address, Vec<u8>, bool, u32)>> {
// 	// // 	handle.record_db_read::<Runtime>(100)?;

// 	// // 	// Convert  to SubscriptionId
// 	// // 	let sub_id: u32 = subscription_id.try_into().map_err(|_| revert("Subscription ID overflow"))?;

// 	// // 	// Get storage requests
// 	// // 	let requests = MarketplacePallet::<Runtime>::get_subscription_storage_requests(sub_id);

// 	// // 	// Convert AccountIds to Addresses
// 	// // 	let converted_requests: Vec<(u32, u32, Address, Vec<u8>, bool, u32)> = requests
// 	// // 		.into_iter()
// 	// // 		.map(|(total, fulfilled, owner, hash, approved, sub_id)| {
// 	// // 			let address: H160 = owner.into();
// 	// // 			(
// 	// // 				total,
// 	// // 				fulfilled,
// 	// // 				Address(address),
// 	// // 				hash,
// 	// // 				approved,
// 	// // 				sub_id
// 	// // 			)
// 	// // 		})
// 	// // 		.collect();

// 	// // 	Ok(converted_requests)
// 	// // }

// 	// #[precompile::public("purchasePackage(uint8,uint32,bytes)")]
// 	// #[precompile::public("purchase_package(uint8,uint32,bytes)")]
// 	// #[precompile::payable]
// 	// fn purchase_package(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	tier: u8,
// 	// 	cdn_location_id: u32,
// 	// 	referral_code: Vec<u8>,
// 	// ) -> EvmResult {
// 	// 	let caller = handle.context().caller;
// 	// 	let value = handle.context().apparent_value;

// 	// 	// Convert u8 to PackageTier
// 	// 	let package_tier = match tier {
// 	// 		0 => PackageTier::Starter,
// 	// 		1 => PackageTier::Quantum,
// 	// 		2 => PackageTier::Pinnacle,
// 	// 		_ => return Err(revert("Invalid package tier"))
// 	// 	};

// 	// 	// Get package price to verify sent value
// 	// 	let package = MarketplacePallet::<Runtime>::get_package(package_tier.clone())
// 	// 		.ok_or_else(|| revert("Package not found"))?;

// 	// 	// Convert cdn_location_id to Option
// 	// 	let cdn_option = if cdn_location_id == 0 {
// 	// 		None
// 	// 	} else {
// 	// 		Some(cdn_location_id)
// 	// 	};

// 	// 	// Convert referral_code to Option<Vec<u8>>
// 	// 	let referral_option = if referral_code.is_empty() {
// 	// 		None
// 	// 	} else {
// 	// 		Some(referral_code.to_vec())
// 	// 	};

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);

// 	// 	// Create and dispatch the call
// 	// 	let call = MarketplaceCall::<Runtime>::purchase_package { 
// 	// 		tier: package_tier,
// 	// 		cdn_location_id: None,
// 	// 		refferal_code: None,
// 	// 		pay_upfront_period_in_months: None,
// 	// 		pay_for: None
// 	// 	};

// 	// 	// Let the pallet handle the token transfer
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// // #[precompile::public("purchasePoints(uint128)")]
// 	// // #[precompile::public("purchase_points(uint128)")]
// 	// // fn purchase_points(
// 	// // 	handle: &mut impl PrecompileHandle,
// 	// // 	amount: u128,
// 	// // ) -> EvmResult {
// 	// // 	let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

// 	// // 	// Convert  to Points (u128)
// 	// // 	let points_amount: u128 = amount
// 	// // 		.try_into()
// 	// // 		.map_err(|_| revert("Amount overflow"))?;

// 	// // 	// Create the call
// 	// // 	let call = MarketplaceCall::<Runtime>::purchase_points { 
// 	// // 		amount: points_amount,
// 	// // 	};

// 	// // 	// Dispatch the call
// 	// // 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// // 		handle,
// 	// // 		Some(origin).into(),
// 	// // 		 call,
// 	// // 	)?;

// 	// // 	Ok(())
// 	// // }

// 	// #[precompile::public("setAutoRenewal(uint32,bool)")]
// 	// #[precompile::public("set_auto_renewal(uint32,bool)")]
// 	// #[precompile::payable]
// 	// fn set_auto_renewal(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	subscription_id: u32,
// 	// 	enabled: bool,
// 	// ) -> EvmResult {
// 	// 	let caller = handle.context().caller;
// 	// 	let value = handle.context().apparent_value;

// 	// 	// Get subscription to verify payment amount
// 	// 	let subscription = MarketplacePallet::<Runtime>::get_user_subscription(
// 	// 		Runtime::AddressMapping::into_account_id(caller),
// 	// 		subscription_id,
// 	// 	).ok_or_else(|| revert("Subscription not found"))?;

// 	// 	// Get package price to verify sent value
// 	// 	let package = MarketplacePallet::<Runtime>::get_package(subscription.package.tier)
// 	// 		.ok_or_else(|| revert("Package not found"))?;
		
// 	// 	// // Verify sent value matches package price if enabling auto-renewal
// 	// 	// if enabled && value != package.price_native.into() {
// 	// 	// 	return Err(revert("Incorrect payment amount"));
// 	// 	// }

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::set_auto_renewal { 
// 	// 		subscription_id,
// 	// 		enabled
// 	// 	};

// 	// 	// Dispatch the call - pallet will handle the payment
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("storageRequest(address,bytes[],uint32)")]
// 	// #[precompile::public("storage_request(address,bytes[],uint32)")]
// 	// fn storage_request(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	owner: Address,
// 	// 	file_hashes: Vec<Vec<u8>>,
// 	// 	subscription_id: u32
// 	// ) -> EvmResult {
// 	// 	// Override gas cost
// 	// 	handle.record_cost(0)?;

// 	// 	let caller = handle.context().caller;
		
// 	// 	// Convert owner Address to AccountId
// 	// 	let owner_address: H160 = owner.into();
// 	// 	let owner_account = Runtime::AddressMapping::into_account_id(owner_address);

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::storage_request { 
// 	// 		owner: owner_account,
// 	// 		file_hashes: file_hashes,
// 	// 		subscription_id: subscription_id,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("storageUnpinRequest(bytes)")]
// 	// #[precompile::public("storage_unpin_request(bytes)")]
// 	// fn storage_unpin_request(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	file_hash: Vec<u8>,
// 	// ) -> EvmResult {
// 	// 	// Override gas cost
// 	// 	handle.record_cost(0)?;

// 	// 	let caller = handle.context().caller;
// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);

// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::storage_unpin_request { 
// 	// 		file_hash: file_hash,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("getSubscriptionStorage(address,uint32)")]
// 	// #[precompile::public("get_subscription_storage(address,uint32)")]
// 	// fn get_subscription_storage(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	user: Address,
// 	// 	subscription_id: u32,
// 	// ) -> EvmResult<(u32, u32)> {  // (total_storage_gb, storage_used)
// 	// 	handle.record_db_read::<Runtime>(100)?;

// 	// 	// Convert address to AccountId
// 	// 	let user_address: H160 = user.into();
// 	// 	let user_account = Runtime::AddressMapping::into_account_id(user_address);

// 	// 	let sub_id: u32 = subscription_id
// 	// 		.try_into()
// 	// 		.map_err(|_| revert("Subscription ID overflow"))?;

// 	// 	// Get subscription details
// 	// 	let subscription = MarketplacePallet::<Runtime>::get_user_subscription(
// 	// 		user_account,
// 	// 		sub_id,
// 	// 	).ok_or_else(|| revert("Subscription not found"))?;

// 	// 	Ok((
// 	// 		subscription.package.storage_gb,  // total storage
// 	// 		subscription.storage_used         // used storage
// 	// 	))
// 	// }

// 	// #[precompile::public("getNextRenewalBlock(address,uint32)")]
// 	// #[precompile::public("get_next_renewal_block(address,uint32)")]
// 	// fn get_next_renewal_block(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	user: Address,
// 	// 	subscription_id: u32,
// 	// ) -> EvmResult<u32> {
// 	// 	handle.record_db_read::<Runtime>(100)?;

// 	// 	// Convert address to AccountId
// 	// 	let user_address: H160 = user.into();
// 	// 	let user_account = Runtime::AddressMapping::into_account_id(user_address);


// 	// 	// Get subscription details
// 	// 	let subscription = MarketplacePallet::<Runtime>::get_user_subscription(
// 	// 		user_account,
// 	// 		subscription_id,
// 	// 	).ok_or_else(|| revert("Subscription not found"))?;

// 	// 	// Check if auto_renewal is enabled
// 	// 	ensure!(
// 	// 		subscription.auto_renewal,
// 	// 		revert("Auto renewal is not enabled for this subscription")
// 	// 	);

// 	// 	// Convert BlockNumber to u32
// 	// 	let next_renewal: u32 = subscription.expires_at
// 	// 		.saturated_into();

// 	// 	Ok(next_renewal)
// 	// }

// 	// #[precompile::public("createReferralCode()")]
// 	// #[precompile::public("create_referral_code()")]
// 	// fn create_referral_code(handle: &mut impl PrecompileHandle) -> EvmResult {
// 	// 	let caller = handle.context().caller;
// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);

// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::create_referral_code {};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("changeReferralCode()")]
// 	// #[precompile::public("change_referral_code()")]
// 	// fn change_referral_code(handle: &mut impl PrecompileHandle) -> EvmResult {
// 	// 	let caller = handle.context().caller;
// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);

// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::change_referral_code {};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("getTotalReferralCodes()")]
// 	// #[precompile::public("get_total_referral_codes()")]
// 	// #[precompile::view]
// 	// fn get_total_referral_codes(handle: &mut impl PrecompileHandle) -> EvmResult<u32> {
// 	// 	let total = pallet_marketplace::Pallet::<Runtime>::get_total_referral_codes();
// 	// 	Ok(total)
// 	// }

// 	// #[precompile::public("getTotalReferralRewards()")]
// 	// #[precompile::public("get_total_referral_rewards()")]
// 	// #[precompile::view]
// 	// fn get_total_referral_rewards(handle: &mut impl PrecompileHandle) -> EvmResult<u128> {
// 	// 	let total = pallet_marketplace::Pallet::<Runtime>::get_total_referral_rewards();
// 	// 	// Convert the Balance type to 
// 	// 	Ok(total.try_into().unwrap_or_default())
// 	// }

// 	// #[precompile::public("getReferralCodeRewards(bytes)")]
// 	// #[precompile::public("get_referral_code_rewards(bytes)")]
// 	// #[precompile::view]
// 	// fn get_referral_code_rewards(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	referral_code: BoundedBytes<ConstU32<32>>,
// 	// ) -> EvmResult<u128> {
// 	// 	// Convert BoundedBytes to Vec<u8>
// 	// 	let code: Vec<u8> = referral_code.into();
		
// 	// 	let rewards = pallet_marketplace::Pallet::<Runtime>::get_referral_code_rewards(code);
// 	// 	// Convert the Balance type to 
// 	// 	Ok(rewards.try_into().unwrap_or_default())
// 	// }

// 	// #[precompile::public("getReferralCodeUsageCount(bytes)")]
// 	// #[precompile::public("get_referral_code_usage_count(bytes)")]
// 	// #[precompile::view]
// 	// fn get_referral_code_usage_count(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	referral_code: BoundedBytes<ConstU32<32>>,
// 	// ) -> EvmResult<u32> {
// 	// 	// Convert BoundedBytes to Vec<u8>
// 	// 	let code: Vec<u8> = referral_code.into();
		
// 	// 	let count = pallet_marketplace::Pallet::<Runtime>::get_referral_code_usage_count(code);
// 	// 	Ok(count)
// 	// }

// 	// #[precompile::public("getPotBalance()")]
// 	// #[precompile::public("get_pot_balance()")]
// 	// #[precompile::view]
// 	// fn get_pot_balance(handle: &mut impl PrecompileHandle) -> EvmResult<u128> {
// 	// 	// Get the pallet's balance
// 	// 	let balance = pallet_marketplace::Pallet::<Runtime>::balance();
		
// 	// 	// Convert the Balance type to  for EVM compatibility
// 	// 	Ok(balance.try_into().unwrap_or_default())
// 	// }

// 	// #[precompile::public("transferSubscription(address,uint32)")]
// 	// #[precompile::public("transfer_subscription(address,uint32)")]
// 	// fn transfer_subscription(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	to: Address,
// 	// 	subscription_id: u32,
// 	// ) -> EvmResult {
// 	// 	let caller = handle.context().caller;
		
// 	// 	// Convert to Address to AccountId
// 	// 	let to_address: H160 = to.into();
// 	// 	let to_account = Runtime::AddressMapping::into_account_id(to_address);

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::transfer_subscription { 
// 	// 		to: to_account,
// 	// 		subscription_id,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("grantAccess(uint32,address,uint8)")]
// 	// #[precompile::public("grant_access(uint32,address,uint8)")]
// 	// fn grant_access(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	subscription_id: u32,
// 	// 	to: Address,
// 	// 	permission: u8,
// 	// ) -> EvmResult {
// 	// 	// Override gas cost
// 	// 	handle.record_cost(0)?;

// 	// 	let caller = handle.context().caller;
		
// 	// 	// Convert to Address to AccountId
// 	// 	let to_address: H160 = to.into();
// 	// 	let to_account = Runtime::AddressMapping::into_account_id(to_address);

// 	// 	// Convert u8 to PermissionLevel
// 	// 	let permission_level = match permission {
// 	// 		0 => PermissionLevel::Read,
// 	// 		1 => PermissionLevel::Write,
// 	// 		2 => PermissionLevel::Admin,
// 	// 		_ => return Err(revert("Invalid permission level"))
// 	// 	};

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::grant_access { 
// 	// 		subscription_id,
// 	// 		to: to_account,
// 	// 		permission: permission_level,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("revokeAccess(uint32,address)")]
// 	// #[precompile::public("revoke_access(uint32,address)")]
// 	// fn revoke_access(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	subscription_id: u32,
// 	// 	from: Address,
// 	// ) -> EvmResult {
// 	// 	// Override gas cost
// 	// 	handle.record_cost(0)?;

// 	// 	let caller = handle.context().caller;
		
// 	// 	// Convert from Address to AccountId
// 	// 	let from_address: H160 = from.into();
// 	// 	let from_account = Runtime::AddressMapping::into_account_id(from_address);

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::revoke_access { 
// 	// 		subscription_id,
// 	// 		from: from_account,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("updateAccessPermission(uint32,address,uint8)")]
// 	// #[precompile::public("update_access_permission(uint32,address,uint8)")]
// 	// fn update_access_permission(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	subscription_id: u32,
// 	// 	user: Address,
// 	// 	new_permission: u8,
// 	// ) -> EvmResult {
// 	// 	let caller = handle.context().caller;
		
// 	// 	// Convert user Address to AccountId
// 	// 	let user_address: H160 = user.into();
// 	// 	let user_account = Runtime::AddressMapping::into_account_id(user_address);

// 	// 	// Convert u8 to PermissionLevel
// 	// 	let permission_level = match new_permission {
// 	// 		0 => PermissionLevel::Read,
// 	// 		1 => PermissionLevel::Write,
// 	// 		2 => PermissionLevel::Admin,
// 	// 		_ => return Err(revert("Invalid permission level"))
// 	// 	};

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::update_access_permission { 
// 	// 		subscription_id,
// 	// 		user: user_account,
// 	// 		new_permission: permission_level,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

// 	// #[precompile::public("switchPackage(uint32,uint8)")]
// 	// #[precompile::public("switch_package(uint32,uint8)")]
// 	// #[precompile::payable]
// 	// fn switch_package(
// 	// 	handle: &mut impl PrecompileHandle,
// 	// 	subscription_id: u32,
// 	// 	new_tier: u8,
// 	// ) -> EvmResult {
// 	// 	let caller = handle.context().caller;
// 	// 	let value = handle.context().apparent_value;
		
// 	// 	// Convert tier to PackageTier
// 	// 	let package_tier = match new_tier {
// 	// 		0 => PackageTier::Starter,
// 	// 		1 => PackageTier::Quantum,
// 	// 		2 => PackageTier::Pinnacle,
// 	// 		_ => return Err(revert("Invalid package tier"))
// 	// 	};

// 	// 	let origin = Runtime::AddressMapping::into_account_id(caller);
		
// 	// 	// Create the call
// 	// 	let call = MarketplaceCall::<Runtime>::switch_package { 
// 	// 		subscription_id,
// 	// 		new_tier: package_tier,
// 	// 	};

// 	// 	// Dispatch the call
// 	// 	RuntimeHelper::<Runtime>::try_dispatch(
// 	// 		handle,
// 	// 		Some(origin).into(),
// 	// 		call,
// 	// 	)?;

// 	// 	Ok(())
// 	// }

//     // /// Upgrade package storage capacity
//     // #[precompile::public("increasePackageStorage(uint32)")]
//     // #[precompile::public("increase_package_storage(uint32)")]
//     // fn increase_package_storage(
//     //     handle: &mut impl PrecompileHandle,
//     //     // subscription_id: u32,
//     //     gbs_needed: u32,
//     // ) -> EvmResult {
//     //     let caller = handle.context().caller;
//     //     let who = Runtime::AddressMapping::into_account_id(caller);

//     //     // Create the call
//     //     let call = MarketplaceCall::<Runtime>::increase_package_storage { 
//     //         // subscription_id,
//     //         gbs_needed,
//     //     };

//     //     // Dispatch the call
//     //     RuntimeHelper::<Runtime>::try_dispatch(
//     //         handle,
//     //         Some(who).into(),
//     //         call,
//     //     )?;

//     //     Ok(())
//     // }
// }