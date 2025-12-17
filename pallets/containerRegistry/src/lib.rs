#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;
mod types;

#[frame_support::pallet]
pub mod pallet {
	use crate::types::*;
	use frame_support::{pallet_prelude::*, Blake2_128Concat};
	use frame_system::pallet_prelude::*;
	use ipfs_pallet::FileInput;
	use sp_std::{vec, vec::Vec};

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_marketplace::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Maximum length for various metadata fields
		#[pallet::constant]
		type MaxLength: Get<u32>;
	}

	#[pallet::storage]
	#[pallet::getter(fn next_space_id)]
	pub type NextSpaceId<T: Config> = StorageValue<_, SpaceId, ValueQuery>;

	/// Storage for Spaces
	#[pallet::storage]
	#[pallet::getter(fn spaces)]
	pub type Spaces<T: Config> =
		StorageMap<_, Blake2_128Concat, SpaceId, SpaceMetadata<T::AccountId>, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn manifest_digests)]
	pub type ManifestDigests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		(Vec<u8>, Vec<u8>, Vec<u8>), // (repo_name, image_name, tag)
		Vec<u8>,                     // digest
		OptionQuery,
	>;

	/// Digest to Info Storage Map
	/// Maps digest to its type and CID
	/// this is for storing blobs
	#[pallet::storage]
	#[pallet::getter(fn digest_info)]
	pub(super) type DigestInfoStorage<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,    // digest
		DigestInfo, // Digest information (type and CID)
		OptionQuery,
	>;

	/// Image Name + Digest to CID Storage Map
	/// Maps (image_name, digest) to cid
	///  this is for storing manifest json
	#[pallet::storage]
	#[pallet::getter(fn image_digest_to_cid)]
	pub(super) type ImageDigestToCid<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		Vec<u8>, // image_name
		Blake2_128Concat,
		Vec<u8>, // digest
		Vec<u8>, // cid
		OptionQuery,
	>;

	/// Digest Information
	#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum DigestType {
		File,
		Json,
	}

	pub type CID = Vec<u8>;

	/// Digest Details
	#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
	pub struct DigestInfo {
		pub digest_type: DigestType, // Type of the digest (file or json)
		pub cid: Vec<u8>,            // CID associated with the digest
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A new space was created [space_id, owner]
		SpaceCreated(SpaceId, T::AccountId),

		/// A member was added to a space [space_id, member]
		MemberAdded(SpaceId, T::AccountId),
		/// A manifest digest was updated [repo_name, image_name, tag, digest]
		ManifestDigestUpdated(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>),
		/// A new mapping of image name + digest to CID was stored
		ImageDigestToCidStored {
			who: T::AccountId,
			image_name: Vec<u8>,
			digest: Vec<u8>,
			cid: Vec<u8>,
		},
		/// Digest information successfully stored
		DigestInfoStored {
			who: T::AccountId,
			digest: Vec<u8>,
			digest_type: DigestType,
			cid: Vec<u8>,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Repository already exists
		RepositoryAlreadyExists,

		/// Maximum tags limit reached
		MaxTagsLimitReached,

		/// Input exceeds maximum allowed length
		ExceedsMaxLength,

		/// Repository not found
		RepositoryNotFound,

		MaxImageCidsLimitReached,

		/// Space already exists
		SpaceAlreadyExists,

		/// Space not found
		SpaceNotFound,

		/// Not authorized to access the space
		NotAuthorized,

		/// Maximum space members limit reached
		MaxSpaceMembersReached,

		/// The image name cannot be empty
		EmptyImageName,

		/// The digest cannot be empty
		EmptyDigest,

		/// The CID cannot be empty
		EmptyCid,

		/// The digest information cannot be empty
		EmptyDigestInfo,

		/// The CID information cannot be empty
		EmptyCidInfo,

		/// Not a member of the space
		NotSpaceMember,

		SpaceDoesNotExist,

		NotSpaceOwner,

		/// User already has a space
		UserAlreadyHasSpace,
	}

	// #[pallet::hooks]
	// impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
	// 	fn on_initialize(_block_number: BlockNumberFor<T>) -> Weight {
	// 		// Retrieve the current list of CID delete requests
	// 		let cid_delete_requests =
	// 			pallet_marketplace::Pallet::<T>::registry_cid_delete_requests();

	// 		// Process each CID delete request
	// 		for cid in cid_delete_requests.clone() {
	// 			// Attempt to delete CID-related entries in container registry
	// 			if let Err(e) = Self::delete_all_cid_related_entries(cid.clone()) {
	// 				log::error!(
	// 					"Failed to delete CID-related entries for CID: {:?}. Error: {:?}",
	// 					cid,
	// 					e
	// 				);
	// 			}
	// 			let _ = pallet_marketplace::Pallet::<T>::remove_cid_delete_request(cid);
	// 		}

	// 		// Return a small weight for the initialization
	// 		Weight::from_parts(10_000, 0)
	// 	}
	// }

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// Create a new space
		#[pallet::call_index(0)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn create_space(origin: OriginFor<T>, name: Vec<u8>) -> DispatchResult {
			let owner = ensure_signed(origin)?;

			// Check if the user already owns a space
			ensure!(
				!Spaces::<T>::iter().any(|(_, space_metadata)| space_metadata.owner == owner),
				Error::<T>::UserAlreadyHasSpace
			);

			// Increment and get the next space ID
			let space_id = NextSpaceId::<T>::get();

			// Add some additional entropy
			let space_id = space_id
				.wrapping_add(frame_system::Pallet::<T>::block_number().try_into().unwrap_or(0));

			// Ensure space doesn't already exist
			ensure!(!Spaces::<T>::contains_key(space_id), Error::<T>::SpaceAlreadyExists);

			// Update next space ID
			NextSpaceId::<T>::put(space_id.wrapping_add(1));

			// Create space metadata
			let space_metadata = SpaceMetadata {
				owner: owner.clone(),
				members: vec![owner.clone()],
				repo_name: name,
				image_names: vec![],
			};

			// Store space
			Spaces::<T>::insert(space_id, space_metadata);

			// Emit event
			Self::deposit_event(Event::SpaceCreated(space_id, owner));

			Ok(())
		}

		/// Add a member to a space
		#[pallet::call_index(1)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_space_member(
			origin: OriginFor<T>,
			space_id: SpaceId,
			new_member: T::AccountId,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			// Retrieve space
			let mut space = Spaces::<T>::get(space_id).ok_or(Error::<T>::SpaceNotFound)?;

			// Ensure only owner can add members
			ensure!(sender == space.owner, Error::<T>::NotAuthorized);

			// Add member
			space.members.push(new_member.clone());

			// Update space
			Spaces::<T>::insert(space_id, space);

			// Emit event
			Self::deposit_event(Event::MemberAdded(space_id, new_member));

			Ok(())
		}

		#[pallet::call_index(2)]
		#[pallet::weight((20_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_manifest_head_digest_and_manifest_json_cid(
			origin: OriginFor<T>,
			repo_name: Vec<u8>,
			image_name: Vec<u8>,
			tag: Option<Vec<u8>>,
			digest: Vec<u8>,
			cid: Vec<u8>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			// Find the space with matching repo_name
			let mut space = Spaces::<T>::iter()
				.find(|(_, space_metadata)| space_metadata.repo_name == repo_name)
				.ok_or(Error::<T>::SpaceDoesNotExist)?
				.1; // Extract the space metadata

			// Ensure caller is the owner of the space
			ensure!(space.owner == who, Error::<T>::NotSpaceOwner);

			// Check if image_name is already in the space's image_names
			if !space.image_names.contains(&image_name) {
				// Add the image_name to the space's image_names
				space.image_names.push(image_name.clone());

				// Update the space in storage
				Spaces::<T>::insert(
					Spaces::<T>::iter()
						.find(|(_, metadata)| metadata.repo_name == repo_name)
						.map(|(id, _)| id)
						.ok_or(Error::<T>::SpaceDoesNotExist)?,
					space,
				);
			}

			// Validations
			ensure!(!image_name.is_empty(), Error::<T>::EmptyImageName);
			ensure!(!digest.is_empty(), Error::<T>::EmptyDigest);
			ensure!(!cid.is_empty(), Error::<T>::EmptyCid);

			// Use default tag if not provided
			let tag = tag.unwrap_or_else(|| b"latest".to_vec());

			// Store manifest digest
			ManifestDigests::<T>::insert(
				(repo_name.clone(), image_name.clone(), tag.clone()),
				digest.clone(),
			);

			// Store CID mapping
			ImageDigestToCid::<T>::insert(&image_name, &digest, &cid);

			// adding req inside marketplace pallet
			let file_input = FileInput {
				file_name: "Image Mainfest Digest".as_bytes().to_vec(),
				file_hash: cid.clone(),
			};

			let _ = pallet_marketplace::Pallet::<T>::process_storage_requests(
				&who.clone(),
				&vec![file_input.clone()],
				None
			);

			// Emit both events
			Self::deposit_event(Event::ManifestDigestUpdated(
				repo_name,
				image_name.clone(),
				tag,
				digest.clone(),
			));

			Self::deposit_event(Event::ImageDigestToCidStored { who, image_name, digest, cid });

			Ok(())
		}

		/// Store digest information (type and CID)
		#[pallet::call_index(3)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn store_digest_info(
			origin: OriginFor<T>,
			repo_name: Vec<u8>,
			digest: Vec<u8>,
			digest_type: DigestType,
			cid: Vec<u8>,
		) -> DispatchResult {
			// Ensure the caller is signed
			let who = ensure_signed(origin)?;

			// Validate inputs
			ensure!(!digest.is_empty(), Error::<T>::EmptyDigest);
			ensure!(!cid.is_empty(), Error::<T>::EmptyCid);

			// Ensure space exists and get owner's space
			let owner_space = Spaces::<T>::iter()
				.find(|(_, space_metadata)| space_metadata.repo_name == repo_name)
				.ok_or(Error::<T>::SpaceNotFound)?;

			// Ensure caller is the owner
			ensure!(owner_space.1.owner == who, Error::<T>::NotSpaceOwner);

			// Create the digest info struct
			let digest_info = DigestInfo { digest_type: digest_type.clone(), cid: cid.clone() };

			// Store in the map
			DigestInfoStorage::<T>::insert(&digest, digest_info);

			let file_input =
				FileInput { file_name: "Digest Info".as_bytes().to_vec(), file_hash: cid.clone() };

			// adding req inside marketplace pallet
			let _ = pallet_marketplace::Pallet::<T>::process_storage_requests(
				&who.clone(),
				&vec![file_input.clone()],
				None,
			);

			// Emit an event for successful storage
			Self::deposit_event(Event::DigestInfoStored { who, digest, digest_type, cid });

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		pub fn get_space_id(repo_name: Vec<u8>) -> Option<SpaceId> {
			Spaces::<T>::iter()
				.find(|(_, space_metadata)| space_metadata.repo_name == repo_name)
				.map(|(space_id, _)| space_id)
		}

		pub fn get_space_cids(account_id: T::AccountId) -> Option<Vec<CID>> {
			// Find the space owned by the given account
			let (_, space) = Spaces::<T>::iter()
				.find(|(_, space_metadata)| space_metadata.owner == account_id)?;

			// Get all manifest digests for this space (repo)
			let repo_name = space.repo_name.clone();

			// Collect ALL CIDs for this repo
			let mut cids: Vec<CID> = Vec::new();

			// Collect CIDs from ManifestDigests
			for ((stored_repo_name, image_name, _), digest) in ManifestDigests::<T>::iter() {
				if stored_repo_name != repo_name {
					continue;
				}

				// Get CIDs from DigestInfoStorage
				if let Some(digest_info) = Self::digest_info(&digest) {
					cids.push(digest_info.cid);
				}

				// Get CIDs from ImageDigestToCid
				if let Some(image_digest_cid) = ImageDigestToCid::<T>::get(image_name, digest) {
					cids.push(image_digest_cid);
				}
			}

			Some(cids)
		}

		/// Delete all storage entries related to a specific CID
		pub fn delete_image_digest_by_cid(cid: Vec<u8>) -> DispatchResult {
			// Collect digests to be deleted from DigestInfoStorage
			let digests_to_delete: Vec<Vec<u8>> = ImageDigestToCid::<T>::iter()
				.filter(|(_, _, stored_cid)| *stored_cid == cid)
				.map(|(_, digest, _)| digest)
				.collect();

			// Remove entries from ImageDigestToCid
			ImageDigestToCid::<T>::iter()
				.filter(|(_, _, stored_cid)| *stored_cid == cid)
				.for_each(|(image_name, digest, _)| {
					ImageDigestToCid::<T>::remove(&image_name, &digest);
				});

			// Remove corresponding entries from DigestInfoStorage
			for digest in digests_to_delete {
				DigestInfoStorage::<T>::remove(&digest);
			}

			Ok(())
		}

		/// Delete all storage entries with the specified CID in DigestInfoStorage
		pub fn delete_digest_info_by_cid(cid: Vec<u8>) -> DispatchResult {
			// Iterate through all entries and remove those with matching CID
			DigestInfoStorage::<T>::iter()
				.filter(|(_, digest_info)| digest_info.cid == cid)
				.for_each(|(digest, _)| {
					DigestInfoStorage::<T>::remove(&digest);
				});

			Ok(())
		}

		/// Comprehensive deletion of all storage entries related to a specific CID
		pub fn delete_all_cid_related_entries(cid: Vec<u8>) -> DispatchResult {
			// Delete entries from ImageDigestToCid
			Self::delete_image_digest_by_cid(cid.clone())?;

			// Delete entries from DigestInfoStorage
			Self::delete_digest_info_by_cid(cid)?;

			Ok(())
		}
	}
}
