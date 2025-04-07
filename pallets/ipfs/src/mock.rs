pub fn handle_request_assignment(
	node_id: Vec<u8>,
	node_info: NodeInfo<BlockNumberFor<T>, T::AccountId>,
) -> Result<(), DispatchError> {
	if AssignmentEnabled::<T>::get() {
		let storage_requests = IpfsPallet::<T>::get_unassigned_storage_requests_for_validator(node_info.owner.clone());
		let active_storage_miners = pallet_registration::Pallet::<T>::get_all_storage_miners_with_min_staked();
		let ranked_list = RankingsPallet::<T>::get_ranked_list();

		let mut rank_map: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
		for ranking in ranked_list.iter() {
			rank_map.insert(ranking.node_id.clone(), ranking.rank);
		}

		for storage_request in storage_requests {
			let mut available_miners: Vec<_> = active_storage_miners
			.iter()
			.filter_map(|miner| {
				// Check if the miner is in a Free state using the IPFS pallet
				let miner_node_id = BoundedVec::try_from(miner.node_id.clone()).ok()?;
				let is_miner_free = IpfsPallet::<T>::is_miner_free(&miner_node_id);
		
				if is_miner_free {
					if let Some(node_metrics) = Self::get_node_metrics(miner.node_id.clone()) {
						let ipfs_storage_max = node_metrics.ipfs_storage_max;
						let available_storage = ipfs_storage_max.saturating_sub(node_metrics.ipfs_repo_size);
						let storage_threshold = ipfs_storage_max.saturating_mul(10) / 100;
		
						if available_storage >= storage_threshold {
							if let Some(rank) = rank_map.get(&miner.node_id) {
								Some((miner.clone(), *rank, available_storage))
							} else {
								None
							}
						} else {
							None
						}
					} else {
						None
					}
				} else {
					None
				}
			})
			.collect();
				
			available_miners.sort_by(|a, b| 
				a.1.cmp(&b.1).then(b.2.cmp(&a.2))
			);

			let mut selected_miners = Vec::new();
			let num_replicas = storage_request.total_replicas.min(available_miners.len() as u32);

			if let Some(requested_miners) = &storage_request.miner_ids {
				for requested_miner_id in requested_miners.iter() {
					if selected_miners.len() >= num_replicas as usize {
						break;
					}
					if let Some((miner, rank, available_storage)) = available_miners.iter().find(|m| m.0.node_id == requested_miner_id.clone().to_vec()) {
						selected_miners.push((miner.clone(), *rank, *available_storage));
					}
				}
			}

			if selected_miners.len() < num_replicas as usize {
				let remaining_needed = num_replicas as usize - selected_miners.len();
				let mut top_miners: Vec<_> = available_miners
					.into_iter()
					.filter(|m| !selected_miners.iter().any(|sm| sm.0.node_id == m.0.node_id))
					.take(remaining_needed)
					.collect();

				let seed = frame_system::Pallet::<T>::block_number().saturated_into::<u64>();
				for i in (1..top_miners.len()).rev() {
					let j = seed.wrapping_mul(i as u64) % (i + 1) as u64;
					top_miners.swap(i, j as usize);
				}
				selected_miners.extend(top_miners);
			}

			if selected_miners.len() == storage_request.total_replicas as usize {
				match IpfsPallet::<T>::fetch_ipfs_file_size(storage_request.file_hash.clone().to_vec()) {
					Ok(file_size) => {
						// Lock all selected miners after they are chosen
						for (miner, _, _) in &selected_miners {
							let miner_node_id = BoundedVec::try_from(miner.node_id.clone())
								.map_err(|_| Error::<T>::StorageOverflow)?;
							
							// Call the set_miner_state_locked function from the IPFS pallet
							IpfsPallet::<T>::call_set_miner_state_locked(miner_node_id);
							
						}
						log::info!("File size: {}", file_size);
						log::info!(
							"Selected {} miners for storage request with file hash {:?}:",
							selected_miners.len(),
							storage_request.file_hash
						);

						let current_block = frame_system::Pallet::<T>::block_number();
						let miner_pin_requests: Vec<MinerPinRequest<BlockNumberFor<T>>> = selected_miners
							.iter()
							.map(|(miner, _, _)| MinerPinRequest {
								miner_node_id: BoundedVec::try_from(miner.node_id.clone()).unwrap(),
								file_hash: storage_request.file_hash.clone(),
								created_at: current_block,
								file_size_in_bytes: file_size,
							})
							.collect();

						for pin_request in miner_pin_requests.iter() {
							let file_hash_vec = pin_request.file_hash.clone().to_vec();
							log::info!(
								"Pinning file for Miner Node ID: {:?}, File Hash: {:?}, File Size: {}",
								pin_request.miner_node_id,
								file_hash_vec,
								pin_request.file_size_in_bytes
							);

							// Check if CID exists in MinerProfile for this miner
							let miner_node_id = pin_request.miner_node_id.clone();
							let existing_cid = MinerProfile::<T>::get(&miner_node_id);

							let json_content = if !existing_cid.is_empty() {
								// Fetch existing content from IPFS
								let binding = existing_cid.to_vec();
								let cid_str = sp_std::str::from_utf8(&binding).map_err(|_| Error::<T>::InvalidCid)?;
									match IpfsPallet::<T>::fetch_ipfs_content(cid_str) {
										Ok(content) => {
											let existing_data: Value = serde_json::from_slice(&content)
												.map_err(|_| Error::<T>::InvalidJson)?;
											let mut requests_array = if existing_data.is_array() {
												existing_data.as_array().unwrap().clone()
											} else {
												vec![existing_data]
											};
											// Append new pin request
											let new_request = serde_json::to_value(pin_request)
												.map_err(|_| Error::<T>::InvalidJson)?;
											requests_array.push(new_request);
											to_string(&requests_array).unwrap()
										}
										Err(e) => {
											log::error!("Failed to fetch existing CID content: {:?}", e);
											// Fallback to creating a new array
											to_string(&vec![pin_request]).unwrap()
										}
									}
							} else {
								// No existing CID, create a new array with this request
								to_string(&vec![pin_request]).unwrap()
							};

							// Pin the updated or new content to IPFS
							match IpfsPallet::<T>::pin_file_to_ipfs(&json_content) {
								Ok(new_cid) => {
									let update_hash: BoundedVec<u8, ConstU32<MAX_FILE_HASH_LENGTH>> =
										BoundedVec::try_from(new_cid.clone().into_bytes())
											.map_err(|_| Error::<T>::StorageOverflow)?;

									let miner_profile_items: Vec<MinerProfileItem> = miner_pin_requests
										.iter()
										.map(|pin_request| MinerProfileItem {
											miner_node_id: pin_request.miner_node_id.clone(),
											cid: update_hash.clone(),
										})
										.collect();

									IpfsPallet::<T>::update_ipfs_request_storage(
										node_id.clone(),
										miner_profile_items,
										storage_request.clone(),
										file_size as u128,
									);
									log::info!("Successfully pinned file with CID: {}", new_cid);
								}
								Err(e) => log::error!("Failed to pin file: {:?}", e),
							}
						}
					}
					Err(e) => {
						log::error!("Failed to fetch file size: {:?}", e);
					}
				}
			}
		}
	}
	Ok(())
}