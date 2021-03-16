#![cfg_attr(not(feature = "std"), no_std)]

use frame_system::{
	self as system,
	ensure_signed,
	offchain::{
		AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer,
	}
};
use frame_support::{
	debug,
	dispatch::DispatchResult, decl_module, decl_storage, decl_event,
};
use sp_core::crypto::KeyTypeId;
use sp_runtime::{
	offchain::{http, Duration},
};
use sp_std::vec::Vec;
use sp_std::cell::RefCell;
use frame_support::traits::IsType;
use sp_runtime::traits::BadOrigin;

#[cfg(test)]
mod tests;

pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"dia!");

pub mod crypto {
	use super::KEY_TYPE;
	use sp_runtime::{
		MultiSignature, MultiSigner,
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
	};
	use sp_core::sr25519::Signature as Sr25519Signature;
	app_crypto!(sr25519, KEY_TYPE);

	pub struct TestAuthId;
	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = Sr25519Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}

pub trait Trait: CreateSignedTransaction<Call<Self>> {
	type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
	type Event: From<Event<Self>> + Into<<Self as frame_system::Trait>::Event>;
	type Call: From<Call<Self>>;
}

decl_storage! {
	trait Store for Module<T: Trait> as DIAOCW {
		DiaData get(fn data): Vec<u8>;
	}
}

decl_event!(
	pub enum Event<T> where AccountId = <T as frame_system::Trait>::AccountId {
		NewDiaData(Vec<u8>, AccountId),
	}
);

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		#[weight = 10_000]
		pub fn submit_data(origin, price: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let body_str = sp_runtime::sp_std::str::from_utf8(&price).map_err(|_| {
				debug::warn!("No UTF8 body");
				BadOrigin
			})?;
			debug::info!("Body: {}", body_str);
			Self::deposit_event(RawEvent::NewDiaData(price, who));
			Ok(())
		}

		fn offchain_worker(block_number: T::BlockNumber) {
			let parent_hash = <system::Module<T>>::block_hash(block_number - 1.into());
			debug::info!("Current block: {:?} (parent hash: {:?})", block_number, parent_hash);

			let res = Self::fetch_data_and_submit_signed();
			if let Err(e) = res {
				debug::error!("Error: {}", e);
			}
		}
	}
}

impl<T: Trait> Module<T> {
	fn fetch_data_and_submit_signed() -> Result<(), &'static str> {
		let signer = Signer::<T, T::AuthorityId>::all_accounts();
		if !signer.can_sign() {
			return Err(
				"No local accounts available. Consider adding one via `author_insertKey` RPC."
			)?
		}
		let data = Self::fetch_data().map_err(|_| "Failed to fetch data")?;
		// We have to borrow the data to capture it in the transaction
		let ref_data = RefCell::new(data);

		// Using `send_signed_transaction` associated type we create and submit a transaction
		// representing the call, we've just created.
		// Submit signed will return a vector of results for all accounts that were found in the
		// local keystore with expected `KEY_TYPE`.
		let results = signer.send_signed_transaction(
			|_account| {
				debug::info!("Submitting fetched data");
				Call::submit_data(ref_data.borrow().into_mut().to_vec())
			}
		);

		for (acc, res) in &results {
			match res {
				Ok(()) => debug::info!("[{:?}] Submitted fetched result", acc.id),
				Err(e) => debug::error!("[{:?}] Failed to submit transaction: {:?}", acc.id, e),
			}
		}

		Ok(())
	}

	fn fetch_data() -> Result<Vec<u8>, http::Error> {
		let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
		let request = http::Request::get(
			"https://api.diadata.org/v1/quotation/BTC"
		);
		let pending = request
			.deadline(deadline)
			.send()
			.map_err(|_| http::Error::IoError)?;

		let response = pending.try_wait(deadline)
			.map_err(|_| http::Error::DeadlineReached)??;
		if response.code != 200 {
			debug::warn!("Unexpected status code: {}", response.code);
			return Err(http::Error::Unknown);
		}

		let body = response.body().collect::<Vec<u8>>();

		// Read body as string and print log, otherwise we only use bytes
		let body_str = sp_runtime::sp_std::str::from_utf8(&body).map_err(|_| {
			debug::warn!("No UTF8 body");
			http::Error::Unknown
		})?;
		debug::warn!("Got response: {}", &body_str);

		Ok(body)
	}
}
