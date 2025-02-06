use sp_runtime::DispatchError;

pub trait SubAccounts<AccountId> {
	fn get_main_account(who: AccountId) -> Result<AccountId, DispatchError>;

	fn add_sub_account(main: AccountId, sub: AccountId) -> Result<(), DispatchError>;

	fn is_sub_account(sender: AccountId, main: AccountId) -> Result<(), DispatchError>;

	fn already_sub_account(who: AccountId) -> Result<(), DispatchError>;
}

pub trait ChargeFees<AccountId> {
	fn get_main_account(who: &AccountId) -> Option<AccountId>;
}
