#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod types;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
	pub use crate::types::*;
	use frame_support::{dispatch::DispatchResultWithPostInfo, pallet_prelude::*};
	use frame_system::pallet_prelude::*;
	use sp_runtime::Vec;

	#[pallet::config]
	pub trait Config: frame_system::Config + scale_info::TypeInfo {
		/// Event type for the runtime
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Cooldown period in blocks
		#[pallet::constant]
		type CooldownPeriod: Get<u32>;
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	/// Storage for notifications
	#[pallet::storage]
	#[pallet::getter(fn notifications)]
	pub type Notifications<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Vec<Notification<T::AccountId, BlockNumberFor<T>>>,
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn banned_accounts)]
	pub type BannedAccounts<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn last_call_time)]
	pub type LastCallTime<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Notification sent (sender, recipient, block number)
		NotificationSent(T::AccountId, T::AccountId, BlockNumberFor<T>),
		/// Notification marked as read (recipient, index)
		NotificationRead(T::AccountId, u32),
		SubscriptionHasEnded {
			recipient: T::AccountId,
			subscription_id: u32,
			expired_at: BlockNumberFor<T>,
		},
		SubscriptionEndingSoon {
			recipient: T::AccountId,
			subscription_id: u32,
			expired_at: BlockNumberFor<T>,
		},
		AccountBanned {
			account: T::AccountId,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		/// No notifications found for the user
		NoNotifications,
		/// Notification index is invalid
		InvalidNotificationIndex,
		CooldownNotElapsed,
		AccountBanned,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Send a notification
		#[pallet::call_index(0)]
		#[pallet::weight((0, Pays::No))]
		pub fn send_notification(
			origin: OriginFor<T>,
			recipient: T::AccountId,
			block_to_send: BlockNumberFor<T>,
			recurrence: bool,
			starting_recurrence: Option<BlockNumberFor<T>>,
			frequency: Option<BlockNumberFor<T>>,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			ensure!(!BannedAccounts::<T>::contains_key(&sender), Error::<T>::AccountBanned);

			// Check for cooldown
			let current_block = <frame_system::Pallet<T>>::block_number();
			if let Some(last_call) = LastCallTime::<T>::get(&sender) {
				ensure!(
					current_block > last_call + T::CooldownPeriod::get().into(),
					Error::<T>::CooldownNotElapsed
				);
			}

			// Proceed to add notification
			let notification = Notification {
				sender: sender.clone(),
				block_to_send,
				recurrence,
				starting_recurrence,
				frequency,
				read: false,
				notification_type: NotificationType::General,
			};

			Notifications::<T>::mutate(&recipient, |notifications| {
				notifications.push(notification);
			});

			// Update the last call time
			LastCallTime::<T>::insert(&sender, current_block);

			Self::deposit_event(Event::NotificationSent(sender, recipient, block_to_send));
			Ok(().into())
		}

		/// Mark a notification as read
		#[pallet::call_index(1)]
		#[pallet::weight((0, Pays::No))]
		pub fn mark_as_read(origin: OriginFor<T>, index: u32) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			Notifications::<T>::try_mutate(&who, |notifications| {
				let notification = notifications
					.get_mut(index as usize)
					.ok_or(Error::<T>::InvalidNotificationIndex)?;

				notification.read = true;

				Self::deposit_event(Event::NotificationRead(who.clone(), index));
				Ok(().into())
			})
		}

		/// Update an existing notification (Sudo only)
		#[pallet::call_index(2)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_update_notification(
			origin: OriginFor<T>,
			recipient: T::AccountId,
			index: u32,
			new_block_to_send: BlockNumberFor<T>,
			new_recurrence: bool,
			new_starting_recurrence: Option<BlockNumberFor<T>>,
			new_frequency: Option<BlockNumberFor<T>>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			Notifications::<T>::try_mutate(&recipient, |notifications| {
				let notification = notifications
					.get_mut(index as usize)
					.ok_or(Error::<T>::InvalidNotificationIndex)?;

				notification.block_to_send = new_block_to_send;
				notification.recurrence = new_recurrence;
				notification.starting_recurrence = new_starting_recurrence;
				notification.frequency = new_frequency;

				Ok(().into())
			})
		}

		#[pallet::call_index(3)]
		#[pallet::weight((0, Pays::No))]
		pub fn ban_account(
			origin: OriginFor<T>,
			account: T::AccountId,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?; // Only root can ban

			BannedAccounts::<T>::insert(account.clone(), ());
			Self::deposit_event(Event::AccountBanned { account });

			Ok(().into())
		}
	}

	/// Helper function to store "subscription has ended" notification
	impl<T: Config> Pallet<T> {
		pub fn notify_subscription_ended(
			sender: T::AccountId,
			recipient: T::AccountId,
			subscription_id: u32,
		) {
			let notification = Notification {
				sender,
				block_to_send: <frame_system::Pallet<T>>::block_number(),
				recurrence: false,
				starting_recurrence: None,
				frequency: None,
				read: false,
				notification_type: NotificationType::SubscriptionHasEnded,
			};

			Notifications::<T>::mutate(&recipient, |notifications| {
				notifications.push(notification);
			});

			Self::deposit_event(Event::SubscriptionHasEnded {
				recipient,
				subscription_id,
				expired_at: <frame_system::Pallet<T>>::block_number(),
			});
		}

		pub fn notify_subscription_ending_soon(
			sender: T::AccountId,
			recipient: T::AccountId,
			subscription_id: u32,
		) {
			let notification = Notification {
				sender,
				block_to_send: <frame_system::Pallet<T>>::block_number(),
				recurrence: false,
				starting_recurrence: None,
				frequency: None,
				read: false,
				notification_type: NotificationType::SubscriptionEndingSoon,
			};

			Notifications::<T>::mutate(&recipient, |notifications| {
				notifications.push(notification);
			});

			Self::deposit_event(Event::SubscriptionEndingSoon {
				recipient,
				subscription_id,
				expired_at: <frame_system::Pallet<T>>::block_number(),
			});
		}
	}
}
