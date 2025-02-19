        fn handle_storage_subscription_charging(current_block: BlockNumberFor<T>) {
            // get total files stores , charge users every hour
            let users_with_buckets = pallet_storage::get_users_with_buckets();
            for user in users_with_buckets {
                // check user last charged_at and if the hour has passed 
                let bucket_names = BucketNames::<T>::get(&user);
                // Track total size for the user's buckets
                let mut user_total_size: u64 = 0;
 
                // Process the bucket names
                for bucket_name in bucket_names {
                    let bucket_name_str = String::from_utf8_lossy(&bucket_name);
 
                    // Perform HTTP request to list bucket contents
                    match pallet_storage::get_bucket_size_in_bytes(&bucket_name_str) {
                        Ok((_response, bucket_size)) => {
                            log::info!(
                                "Bucket {} size: {} bytes",
                                bucket_name_str,
                                bucket_size
                            );
                            // Accumulate total size for the user
                            user_total_size += bucket_size;
                        },
                        Err(err) => {
                            log::error!(
                                "Failed to list contents for bucket {}: {:?}",
                                bucket_name_str,
                                err
                            );
                        }
                    }
                }

                // Skip if no files to charge
                if user_total_size == 0 {
                    continue;
                }

                // Convert total file size to gigabytes
                let total_file_size_in_gbs = user_total_size as f64 / 1_073_741_824.0;

                // Get the current price per GB from the marketplace pallet
                let price_per_gb = Self::get_price_per_gb();
                
                let user_free_credits = CreditsPallet::<T>::get_free_credits(&user);
                
                // Round up to the nearest whole number of GBs
                let rounded_gbs = ((total_file_size_in_gbs).floor() as u128) + 1;
                let charge_amount = price_per_gb * rounded_gbs;                    

                if user_free_credits >= charge_amount {
                    // Decrease user credits
                    CreditsPallet::<T>::decrease_user_credits(&user, charge_amount);

                    // Handle referral logic
                    let mut total_discount = 0u128;
                    if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&user) {
                        let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, charge_amount, &mut total_discount);
                    }
                    
                    // Calculate the amounts for Compute Rankings and Marketplace
                    let total_charge = charge_amount - total_discount;
                    let rankings_amount = total_charge
                        .checked_mul(70u32.into())
                        .and_then(|x| x.checked_div(100u32.into()))
                        .unwrap_or_default();
                                
                    let marketplace_amount = total_charge - rankings_amount;

                    // Mint 70% to Compute Rankings
                    let _ = pallet_balances::Pallet::<T>::deposit_creating(
                        &RankingsPallet::<T>::account_id(), 
                        rankings_amount.try_into().unwrap_or_default()
                    );

                    // Deposit remaining 30% amount to marketplace account
                    let _ = pallet_balances::Pallet::<T>::deposit_creating(
                        &Self::account_id(), 
                        marketplace_amount.try_into().unwrap_or_default()
                    );

                    // Record transaction
                    let _ = Self::record_native_transaction(
                        &user,
                        NativeTransactionType::Subscription,
                        (charge_amount - total_discount).into(),
                    );

                    // Update last charged block for each request
                    
                } else {
                    // Get the current block number
                    let current_block = frame_system::Pallet::<T>::block_number();

                    let blocks_per_hour = T::BlocksPerHour::get();
                    let grace_period_blocks = T::StorageGracePeriod::get();
                    
                    // Calculate grace period start after hourly charging
                    let grace_period_start = subscription.last_charged_at.saturating_add(blocks_per_hour.into());
                    
                    // Check if the current block is within the grace period
                    if current_block.saturating_sub(grace_period_start) <= grace_period_blocks.into() {
                        // Still within grace period
                        log::info!(
                            "Storage request for user {:?} is in grace period",
                            user
                        );
                    } else {
                        // Cancel the request after grace period
                        // and delete storage 
                    }

                }

            }
        }
