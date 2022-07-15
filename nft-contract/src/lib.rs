use near_contract_standards::non_fungible_token::core::{
    NonFungibleTokenCore, NonFungibleTokenResolver,
};
use near_contract_standards::non_fungible_token::metadata::{
    NFTContractMetadata, NonFungibleTokenMetadataProvider, TokenMetadata, NFT_METADATA_SPEC,
};
use near_contract_standards::non_fungible_token::NonFungibleToken;
use near_contract_standards::non_fungible_token::{Token, TokenId};
use near_sdk::Metadata;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LazyOption, LookupMap, UnorderedMap, UnorderedSet, Vector};
use near_sdk::json_types::{ValidAccountId, U128, U64};
use near_sdk::{
    assert_one_yocto, env, near_bindgen, serde_json::json, AccountId, Balance, BorshStorageKey,
    PanicOnDefault, Promise, PromiseOrValue, Gas, ext_contract,
};
use near_sdk::serde::{Deserialize, Serialize};
use std::collections::{HashMap};
use near_sdk::env::{is_valid_account_id};

pub mod event;

pub use event::NearEvent;

/// between token_series_id and edition number e.g. 42:2 where 42 is series and 2 is edition
pub const TOKEN_DELIMETER: char = ':';
/// TokenMetadata.title returned for individual token e.g. "Title — 2/10" where 10 is max copies
pub const TITLE_DELIMETER: &str = " #";
/// e.g. "Title — 2/10" where 10 is max copies
pub const EDITION_DELIMETER: &str = "/";
pub const TREASURY_FEE: u128 = 500; // 500 / 10_000 = 0.05

const GAS_FOR_RESOLVE_TRANSFER: Gas = 10_000_000_000_000;
const GAS_FOR_NFT_TRANSFER_CALL: Gas = 30_000_000_000_000 + GAS_FOR_RESOLVE_TRANSFER;
const GAS_FOR_NFT_APPROVE: Gas = 10_000_000_000_000;
const GAS_FOR_MINT: Gas = 90_000_000_000_000;
const NO_DEPOSIT: Balance = 0;

pub type TokenSeriesId = String;
pub type MintBundleId = String;

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Payout {
    pub payout: HashMap<AccountId, U128>,
}

#[ext_contract(ext_non_fungible_token_receiver)]
trait NonFungibleTokenReceiver {
    /// Returns `true` if the token should be returned back to the sender.
    fn nft_on_transfer(
        &mut self,
        sender_id: AccountId,
        previous_owner_id: AccountId,
        token_id: TokenId,
        msg: String,
    ) -> Promise;
}

#[ext_contract(ext_approval_receiver)]
pub trait NonFungibleTokenReceiver {
    fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: AccountId,
        approval_id: u64,
        msg: String,
    );
}

#[ext_contract(ext_self)]
trait NonFungibleTokenResolver {
    fn nft_resolve_transfer(
        &mut self,
        previous_owner_id: AccountId,
        receiver_id: AccountId,
        token_id: TokenId,
        approved_account_ids: Option<HashMap<AccountId, u64>>,
    ) -> bool;
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct TokenSeries {
    metadata: TokenMetadata,
    creator_id: AccountId,
    tokens: UnorderedSet<TokenId>,
    price: Option<Balance>,
    is_mintable: bool,
    royalty: HashMap<AccountId, u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct TokenSeriesJson {
    token_series_id: TokenSeriesId,
    metadata: TokenMetadata,
    creator_id: AccountId,
    royalty: HashMap<AccountId, u32>,
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct MintBundle {
    token_series_ids: Option<Vector<TokenSeriesId>>,
    token_ids: Option<Vector<TokenId>>,
    price: Option<Balance>,
    limit_buy: Option<u32>,
    bought_account_ids: LookupMap<AccountId, u32>,
}

#[derive(Serialize, Deserialize)]
pub struct MintBundleJson {
    token_series_ids: Option<Vec<TokenSeriesId>>,
    token_ids: Option<Vec<TokenId>>,
    price: Option<U128>,
    limit_buy: Option<u32>,
}

near_sdk::setup_alloc!();

#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct ContractV1 {
    tokens: NonFungibleToken,
    metadata: LazyOption<NFTContractMetadata>,
    // CUSTOM
    token_series_by_id: UnorderedMap<TokenSeriesId, TokenSeries>,
    treasury_id: AccountId,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    tokens: NonFungibleToken,
    metadata: LazyOption<NFTContractMetadata>,
    // CUSTOM
    token_series_by_id: UnorderedMap<TokenSeriesId, TokenSeries>,
    treasury_id: AccountId,
    mint_bundles: UnorderedMap<MintBundleId, MintBundle>,
}

const DATA_IMAGE_SVG_COMIC_ICON: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI2MyIgdmlld0JveD0iMCAwIDQ3LjI1IDQ3LjI1IiBoZWlnaHQ9IjYzIiB2ZXJzaW9uPSIxLjAiPjxkZWZzPjxjbGlwUGF0aCBpZD0iYSI+PHBhdGggZD0iTTEuMTcyIDBoNDYuNTEydjQ2LjM3OUgxLjE3MlptMCAwIi8+PC9jbGlwUGF0aD48L2RlZnM+PGcgY2xpcC1wYXRoPSJ1cmwoI2EpIj48cGF0aCBkPSJNMjQuNDI2LjEyMWMtMTIuNzAzIDAtMjMgMTAuMy0yMyAyMy4wMDQgMCAxMi43MDMgMTAuMjk3IDIzLjAwNCAyMyAyMy4wMDQgMTIuNzA3IDAgMjMuMDA0LTEwLjMgMjMuMDA0LTIzLjAwNEM0Ny40MyAxMC40MjIgMzcuMTMzLjEyMSAyNC40MjYuMTIxIi8+PC9nPjxwYXRoIGZpbGw9IiNGRkYiIGQ9Ik0yNC40MjIgMS40MTRjLTExLjk4OCAwLTIxLjcwNyA5LjcxOS0yMS43MDcgMjEuNzAzIDAgMTEuOTg4IDkuNzE5IDIxLjcwNyAyMS43MDcgMjEuNzA3IDExLjk4OCAwIDIxLjcwNy05LjcxOSAyMS43MDctMjEuNzA3IDAtMTEuOTg0LTkuNzE5LTIxLjcwMy0yMS43MDctMjEuNzAzIi8+PHBhdGggZD0iTTI0LjQxOCAyLjYwNWMtMTEuMzI4IDAtMjAuNTEyIDkuMTg0LTIwLjUxMiAyMC41MDggMCAxMS4zMzIgOS4xODQgMjAuNTEyIDIwLjUxMiAyMC41MTIgMTEuMzI4IDAgMjAuNTEyLTkuMTggMjAuNTEyLTIwLjUxMiAwLTExLjMyNC05LjE4NC0yMC41MDgtMjAuNTEyLTIwLjUwOCIvPjxwYXRoIGZpbGw9IiNGRkYiIGQ9Ik0xMC42MDIgMjQuNjRoLTIuNjNjLS4xMDkgMC0uMTk5LS4wODUtLjE5OS0uMTk1di0yLjYzM2EuMi4yIDAgMCAxIC4yLS4xOTVoMi42MjljLjEwOSAwIC4xOTkuMDkuMTk5LjE5NXYyLjYzM2MwIC4xMS0uMDkuMTk2LS4yLjE5Nk0xOS4xNiAyNS4zODdoLTMuOTNhLjI5NS4yOTUgMCAwIDEtLjI5My0uMjkzdi0zLjkzYzAtLjE2NC4xMzMtLjI5My4yOTMtLjI5M2gzLjkzYy4xNiAwIC4yOTMuMTI5LjI5My4yOTN2My45M2MwIC4xNi0uMTMzLjI5My0uMjkzLjI5M00yOS4xMjkgMjYuMDgyaC01LjE1MmEuMzguMzggMCAwIDEtLjM4My0uMzgzdi01LjE1MmMwLS4yMTEuMTcyLS4zODMuMzgzLS4zODNoNS4xNTJjLjIxIDAgLjM4My4xNzIuMzgzLjM4M3Y1LjE1MmEuMzguMzggMCAwIDEtLjM4My4zODNNNDAuNTkgMjYuODI4aC02LjQ1YS40ODYuNDg2IDAgMCAxLS40OC0uNDg0di02LjQ1YzAtLjI2MS4yMTktLjQ4LjQ4LS40OGg2LjQ1Yy4yNjUgMCAuNDguMjE5LjQ4LjQ4djYuNDVhLjQ4My40ODMgMCAwIDEtLjQ4LjQ4NCIvPjwvc3ZnPg==";

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    NonFungibleToken,
    Metadata,
    TokenMetadata,
    Enumeration,
    Approval,
    // CUSTOM
    TokenSeriesById,
    TokensBySeriesInner { token_series: String },
    TokensPerOwner { account_hash: Vec<u8> },
    MintBundles,
    BoughtAccountId { mint_bundle_id: MintBundleId },
    MintBundleTokens { mint_bundle_id: MintBundleId },
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new_default_meta(owner_id: ValidAccountId, treasury_id: ValidAccountId) -> Self {
        Self::new(
            owner_id,
            treasury_id,
            NFTContractMetadata {
                spec: NFT_METADATA_SPEC.to_string(),
                name: "Marbledao".to_string(),
                symbol: "Marbledao".to_string(),
                icon: Some(DATA_IMAGE_SVG_COMIC_ICON.to_string()),
                base_uri: Some("https://marbledao.mypinata.cloud/ipfs".to_string()),
                reference: None,
                reference_hash: None,
            },
        )
    }

    #[init]
    pub fn new(
        owner_id: ValidAccountId,
        treasury_id: ValidAccountId,
        metadata: NFTContractMetadata,
    ) -> Self {
        assert!(!env::state_exists(), "Already initialized");
        metadata.assert_valid();
        Self {
            tokens: NonFungibleToken::new(
                StorageKey::NonFungibleToken,
                owner_id,
                Some(StorageKey::TokenMetadata),
                Some(StorageKey::Enumeration),
                Some(StorageKey::Approval),
            ),
            token_series_by_id: UnorderedMap::new(StorageKey::TokenSeriesById),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
            treasury_id: treasury_id.to_string(),
            mint_bundles: UnorderedMap::new(StorageKey::MintBundles),
        }
    }

    #[init(ignore_state)]
    pub fn migrate() -> Self {
        let prev: ContractV1 = env::state_read().expect("ERR_NOT_INITIALIZED");
        assert_eq!(
            env::predecessor_account_id(),
            prev.tokens.owner_id,
            "Marble: Only owner"
        );

        let this = Contract {
            tokens: prev.tokens,
            metadata: prev.metadata,
            token_series_by_id: prev.token_series_by_id,
            treasury_id: prev.treasury_id,
            mint_bundles: UnorderedMap::new(StorageKey::MintBundles),
        };

        this
    }

    // Treasury
    #[payable]
    pub fn set_treasury(&mut self, treasury_id: ValidAccountId) {
        assert_one_yocto();
        assert_eq!(
            env::predecessor_account_id(),
            self.tokens.owner_id,
            "Marble: Owner only"
        );
        self.treasury_id = treasury_id.to_string();
    }

    // CUSTOM

    #[payable]
    pub fn nft_create_series(
        &mut self,
        token_metadata: TokenMetadata,
        price: Option<U128>,
        royalty: Option<HashMap<AccountId, u32>>,
        creator_id: ValidAccountId,
    ) -> TokenSeriesJson {
        let initial_storage_usage = env::storage_usage();

        // assert_eq!(env::predecessor_account_id(), self.tokens.owner_id, "Marble: Only owner");

        let token_series_id = format!("{}", (self.token_series_by_id.len() + 1));

        assert!(
            self.token_series_by_id.get(&token_series_id).is_none(),
            "Marble: duplicate token_series_id"
        );

        let title = token_metadata.title.clone();
        assert!(title.is_some(), "Marble: token_metadata.title is required");


        let mut total_perpetual = 0;
        let mut total_accounts = 0;
        let royalty_res: HashMap<AccountId, u32> = if let Some(royalty) = royalty {
            for (k, v) in royalty.iter() {
                if !is_valid_account_id(k.as_bytes()) {
                    env::panic("Not valid account_id for royalty".as_bytes());
                };
                total_perpetual += *v;
                total_accounts += 1;
            }
            royalty
        } else {
            HashMap::new()
        };

        assert!(total_accounts <= 10, "Marble: royalty exceeds 10 accounts");

        assert!(
            total_perpetual <= 9000,
            "Marble Exceeds maximum royalty -> 9000",
        );

        let price_res: Option<u128> = if price.is_some() {
            Some(price.unwrap().0)
        } else {
            None
        };

        self.token_series_by_id.insert(&token_series_id, &TokenSeries {
            metadata: token_metadata.clone(),
            creator_id: creator_id.to_string(),
            tokens: UnorderedSet::new(
                StorageKey::TokensBySeriesInner {
                    token_series: token_series_id.clone(),
                }
                    .try_to_vec()
                    .unwrap(),
            ),
            price: price_res,
            is_mintable: true,
            royalty: royalty_res.clone(),
        });

        env::log(
            json!({
                "type": "nft_create_series",
                "params": {
                    "token_series_id": token_series_id,
                    "token_metadata": token_metadata,
                    "creator_id": creator_id,
                    "price": price,
                    "royalty": royalty_res
                }
            })
                .to_string()
                .as_bytes(),
        );

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);

        TokenSeriesJson {
            token_series_id,
            metadata: token_metadata,
            creator_id: creator_id.into(),
            royalty: royalty_res,
        }
    }

    #[payable]
    pub fn nft_buy(
        &mut self,
        token_series_id: TokenSeriesId,
        receiver_id: ValidAccountId,
        nft_metadata: Option<TokenMetadata>,
    ) -> TokenId {
        let initial_storage_usage = env::storage_usage();

        let token_series = self.token_series_by_id.get(&token_series_id).expect("Marble: Token series not exist");
        let price: u128 = token_series.price.expect("Marble: not for sale");
        let attached_deposit = env::attached_deposit();
        assert!(
            attached_deposit >= price,
            "Marble: attached deposit is less than price : {}",
            price
        );
        let token_id: TokenId = self._nft_mint_series(token_series_id, receiver_id.to_string(), nft_metadata);

        let for_treasury = price as u128 * TREASURY_FEE / 10_000u128;
        let price_deducted = price - for_treasury;
        Promise::new(token_series.creator_id).transfer(price_deducted);
        Promise::new(self.treasury_id.clone()).transfer(for_treasury);

        refund_deposit(env::storage_usage() - initial_storage_usage, price);

        NearEvent::log_nft_mint(
            receiver_id.to_string(),
            vec![token_id.clone()],
            Some(json!({"price": price.to_string()}).to_string()),
        );

        token_id
    }

    #[payable]
    pub fn nft_mint(
        &mut self,
        token_series_id: TokenSeriesId,
        receiver_id: ValidAccountId,
        nft_metadata: Option<TokenMetadata>,
    ) -> TokenId {
        let initial_storage_usage = env::storage_usage();

        let token_series = self.token_series_by_id.get(&token_series_id).expect("Marble: Token series not exist");
        assert_eq!(env::predecessor_account_id(), token_series.creator_id, "Marble: not creator");
        let token_id: TokenId = self._nft_mint_series(token_series_id, receiver_id.to_string(),nft_metadata);

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);

        NearEvent::log_nft_mint(
            receiver_id.to_string(),
            vec![token_id.clone()],
            None,
        );

        token_id
    }

    #[payable]
    pub fn nft_mint_and_approve(
        &mut self,
        token_series_id: TokenSeriesId,
        account_id: ValidAccountId,
        nft_metadata:Option<TokenMetadata>,
        msg: Option<String>,
    ) -> Option<Promise> {
        let initial_storage_usage = env::storage_usage();

        let token_series = self.token_series_by_id.get(&token_series_id).expect("Marble: Token series not exist");
        assert_eq!(env::predecessor_account_id(), token_series.creator_id, "Marble: not creator");
        let token_id: TokenId = self._nft_mint_series(token_series_id, token_series.creator_id.clone(),nft_metadata);

        // Need to copy the nft_approve code here to solve the gas problem
        // get contract-level LookupMap of token_id to approvals HashMap
        let approvals_by_id = self.tokens.approvals_by_id.as_mut().unwrap();

        // update HashMap of approvals for this token
        let approved_account_ids =
            &mut approvals_by_id.get(&token_id).unwrap_or_else(|| HashMap::new());
        let account_id: AccountId = account_id.into();
        let approval_id: u64 =
            self.tokens.next_approval_id_by_id.as_ref().unwrap().get(&token_id).unwrap_or_else(|| 1u64);
        approved_account_ids.insert(account_id.clone(), approval_id);

        // save updated approvals HashMap to contract's LookupMap
        approvals_by_id.insert(&token_id, &approved_account_ids);

        // increment next_approval_id for this token
        self.tokens.next_approval_id_by_id.as_mut().unwrap().insert(&token_id, &(approval_id + 1));

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);

        NearEvent::log_nft_mint(
            token_series.creator_id.clone(),
            vec![token_id.clone()],
            None,
        );

        if let Some(msg) = msg {
            Some(ext_approval_receiver::nft_on_approve(
                token_id,
                token_series.creator_id,
                approval_id,
                msg,
                &account_id,
                NO_DEPOSIT,
                env::prepaid_gas() - GAS_FOR_NFT_APPROVE - GAS_FOR_MINT,
            ))
        } else {
            None
        }
    }

    fn _nft_mint_series(
        &mut self,
        token_series_id: TokenSeriesId,
        receiver_id: AccountId,
        nft_metadata: Option<TokenMetadata>,
    ) -> TokenId {
        let mut token_series = self.token_series_by_id.get(&token_series_id).expect("Marble: Token series not exist");
        assert!(
            token_series.is_mintable,
            "Marble: Token series is not mintable"
        );

        let num_tokens = token_series.tokens.len();
        let max_copies = token_series.metadata.copies.unwrap_or(u64::MAX);
        assert!(num_tokens < max_copies, "Series supply maxed");

        if (num_tokens + 1) >= max_copies {
            token_series.is_mintable = false;
        }

        let token_id = format!("{}{}{}", &token_series_id, TOKEN_DELIMETER, num_tokens + 1);
        token_series.tokens.insert(&token_id);
        self.token_series_by_id.insert(&token_series_id, &token_series);

        // you can add custom metadata to each token here

        let _metadata: TokenMetadata = match nft_metadata {
            None => {
                TokenMetadata {
                    title: None,          // ex. "Arch Nemesis: Mail Carrier" or "Parcel #5055"
                    description: None,    // free-form description
                    media: None, // URL to associated media, preferably to decentralized, content-addressed storage
                    media_hash: None, // Base64-encoded sha256 hash of content referenced by the `media` field. Required if `media` is included.
                    copies: None, // number of copies of this set of metadata in existence when token was minted.
                    issued_at: Some(env::block_timestamp().to_string()), // ISO 8601 datetime when token was issued or minted
                    expires_at: None, // ISO 8601 datetime when token expires
                    starts_at: None, // ISO 8601 datetime when token starts being valid
                    updated_at: None, // ISO 8601 datetime when token was last updated
                    extra: None, // anything extra the NFT wants to store on-chain. Can be stringified JSON.
                    reference: None, // URL to an off-chain JSON file with more info.
                    reference_hash: None, // Base64-encoded sha256 hash of JSON from reference field. Required if `reference` is included.
                }
            },
            Some(metadata) => {
                metadata
            },
        };

        //let token = self.tokens.mint(token_id, receiver_id, metadata);
        // From : https://github.com/near/near-sdk-rs/blob/master/near-contract-standards/src/non_fungible_token/core/core_impl.rs#L359
        // This allows lazy minting

        let owner_id: AccountId = receiver_id;
        self.tokens.owner_by_id.insert(&token_id, &owner_id);

        self.tokens
            .token_metadata_by_id
            .as_mut()
            .and_then(|by_id| by_id.insert(&token_id, &_metadata));

        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let mut token_ids = tokens_per_owner.get(&owner_id).unwrap_or_else(|| {
                UnorderedSet::new(StorageKey::TokensPerOwner {
                    account_hash: env::sha256(&owner_id.as_bytes()),
                })
            });
            token_ids.insert(&token_id);
            tokens_per_owner.insert(&owner_id, &token_ids);
        }


        token_id
    }

    #[payable]
    pub fn nft_set_series_non_mintable(&mut self, token_series_id: TokenSeriesId) {
        assert_one_yocto();

        let mut token_series = self.token_series_by_id.get(&token_series_id).expect("Token series not exist");
        assert_eq!(
            env::predecessor_account_id(),
            token_series.creator_id,
            "Marble: Creator only"
        );

        assert_eq!(
            token_series.is_mintable,
            true,
            "Marble: already non-mintable"
        );

        assert_eq!(
            token_series.metadata.copies,
            None,
            "Marble: decrease supply if copies not null"
        );

        token_series.is_mintable = false;
        self.token_series_by_id.insert(&token_series_id, &token_series);
        env::log(
            json!({
                "type": "nft_set_series_non_mintable",
                "params": {
                    "token_series_id": token_series_id,
                }
            })
                .to_string()
                .as_bytes(),
        );
    }

    #[payable]
    pub fn nft_decrease_series_copies(
        &mut self,
        token_series_id: TokenSeriesId,
        decrease_copies: U64,
    ) -> U64 {
        assert_one_yocto();

        let mut token_series = self.token_series_by_id.get(&token_series_id).expect("Token series not exist");
        assert_eq!(
            env::predecessor_account_id(),
            token_series.creator_id,
            "Marble: Creator only"
        );

        let minted_copies = token_series.tokens.len();
        let copies = token_series.metadata.copies.unwrap();

        assert!(
            (copies - decrease_copies.0) >= minted_copies,
            "Marble: cannot decrease supply, already minted : {}", minted_copies
        );

        let is_non_mintable = if (copies - decrease_copies.0) == minted_copies {
            token_series.is_mintable = false;
            true
        } else {
            false
        };

        token_series.metadata.copies = Some(copies - decrease_copies.0);

        self.token_series_by_id.insert(&token_series_id, &token_series);
        env::log(
            json!({
                "type": "nft_decrease_series_copies",
                "params": {
                    "token_series_id": token_series_id,
                    "copies": U64::from(token_series.metadata.copies.unwrap()),
                    "is_non_mintable": is_non_mintable,
                }
            })
                .to_string()
                .as_bytes(),
        );
        U64::from(token_series.metadata.copies.unwrap())
    }

    #[payable]
    pub fn nft_set_series_price(&mut self, token_series_id: TokenSeriesId, price: Option<U128>) -> Option<U128> {
        assert_one_yocto();

        let mut token_series = self.token_series_by_id.get(&token_series_id).expect("Token series not exist");
        assert_eq!(
            env::predecessor_account_id(),
            token_series.creator_id,
            "Marble: Creator only"
        );

        assert_eq!(
            token_series.is_mintable,
            true,
            "Marble: token series is not mintable"
        );

        if price.is_none() {
            token_series.price = None;
        } else {
            token_series.price = Some(price.unwrap().0);
        }

        self.token_series_by_id.insert(&token_series_id, &token_series);
        env::log(
            json!({
                "type": "nft_set_series_price",
                "params": {
                    "token_series_id": token_series_id,
                    "price": price
                }
            })
                .to_string()
                .as_bytes(),
        );
        return price;
    }

    #[payable]
    pub fn nft_change_metadata(&mut self, token_id: TokenId, metadata: TokenMetadata) {
        assert_one_yocto();
        let owner_id = self.tokens.owner_by_id.get(&token_id).unwrap();
        assert_eq!(
            owner_id,
            env::predecessor_account_id(),
            "Token owner only"
        );
        // let mut token_metadata = self.tokens.token_metadata_by_id.as_ref().unwrap().get(&token_id).unwrap();
        let new_metadata = Some(metadata);
        self.tokens
            .token_metadata_by_id
            .as_mut()
            .and_then(|by_id| by_id.insert(&token_id, &new_metadata.as_ref().unwrap()));
        
    }

    #[payable]
    pub fn nft_burn(&mut self, token_id: TokenId) {
        assert_one_yocto();

        let owner_id = self.tokens.owner_by_id.get(&token_id).unwrap();
        assert_eq!(
            owner_id,
            env::predecessor_account_id(),
            "Token owner only"
        );

        if let Some(next_approval_id_by_id) = &mut self.tokens.next_approval_id_by_id {
            next_approval_id_by_id.remove(&token_id);
        }

        if let Some(approvals_by_id) = &mut self.tokens.approvals_by_id {
            approvals_by_id.remove(&token_id);
        }

        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let mut token_ids = tokens_per_owner.get(&owner_id).unwrap();
            token_ids.remove(&token_id);
            tokens_per_owner.insert(&owner_id, &token_ids);
        }

        if let Some(token_metadata_by_id) = &mut self.tokens.token_metadata_by_id {
            token_metadata_by_id.remove(&token_id);
        }

        self.tokens.owner_by_id.remove(&token_id);

        NearEvent::log_nft_burn(
            owner_id,
            vec![token_id],
            None,
            None,
        );
    }

    // Mint Bundles

    #[payable]
    pub fn buy_mint_bundle(
        &mut self,
        mint_bundle_id: MintBundleId,
        receiver_id: ValidAccountId,
    ) -> Option<TokenId> {
        let initial_storage_usage = env::storage_usage();
        let mut mint_bundle = self.mint_bundles.get(&mint_bundle_id).expect(
            "Marble: Mint bundle does not exist or already finished"
        );

        assert_eq!(env::predecessor_account_id(), receiver_id.to_string(), "Marble: Can only buy for caller");

        let price = mint_bundle.price.expect("Marble: Mint bundle hasn't started yet");
        assert!(
            env::attached_deposit() > price,
            "Marble: Attached deposit lower than mint price"
        );

        if let Some(limit_buy) = mint_bundle.limit_buy {
            let mint_count = mint_bundle.bought_account_ids.get(&receiver_id.to_string()).unwrap_or(0);
            assert!(
                mint_count < limit_buy,
                "Marble: Mint exhausted for account_id {}",
                receiver_id
            );

            mint_bundle.bought_account_ids.insert(
                &receiver_id.to_string(),
                &(mint_count + 1)
            );
        }

        return if let Some(mut token_series_ids) = mint_bundle.token_series_ids {
            let seed_num = get_random_number(0) as u64;
            let index = seed_num % token_series_ids.len();
            let token_series_id = token_series_ids.get(index).unwrap();
            let token_id = self._nft_mint_series(token_series_id.clone(), receiver_id.to_string(),None);

            let token_series = self.token_series_by_id.get(&token_series_id.to_string()).unwrap();
            if !token_series.is_mintable {
                token_series_ids.swap_remove(index);
            }

            if token_series_ids.len() == 0 {
                self.mint_bundles.remove(&mint_bundle_id);
            } else {
                mint_bundle.token_series_ids = Some(token_series_ids);
                self.mint_bundles.insert(&mint_bundle_id, &mint_bundle);
            }

            if price > 0 {
                let for_treasury = price as u128 * TREASURY_FEE / 10_000u128;
                let price_deducted = price - for_treasury;
                Promise::new(token_series.creator_id).transfer(price_deducted);
                Promise::new(self.treasury_id.clone()).transfer(for_treasury);
            }

            refund_deposit(env::storage_usage() - initial_storage_usage, price);

            NearEvent::log_nft_mint(
                receiver_id.to_string(),
                vec![token_id.clone()],
                Some(json!({"price": price.to_string()}).to_string()),
            );

            Some(token_id)
        } else {
            None
        }
    }

    #[payable]
    pub fn create_mint_bundle(
        &mut self,
        mint_bundle_id: MintBundleId,
        token_series_ids: Option<Vec<TokenSeriesId>>,
        token_ids: Option<Vec<TokenId>>,
        price: Option<U128>,
        limit_buy: Option<u32>,
    ) -> bool {
        let initial_storage_usage = env::storage_usage();

        assert_eq!(env::predecessor_account_id(), self.tokens.owner_id, "Marble: Only owner");

        let mint_bundle = self.mint_bundles.get(&mint_bundle_id);

        if mint_bundle.is_some() {
            panic!("Mint bundle already exists");
        }

        if token_series_ids.is_some() {
            assert!(
                token_ids.is_none(),
                "Must chose either token_series_ids or token_ids"
            );
            let mut token_series_ids_internal: Vector<TokenSeriesId> = Vector::new(
                StorageKey::MintBundleTokens { mint_bundle_id: mint_bundle_id.clone() }
            );
            for token_series_id in token_series_ids.unwrap() {
                token_series_ids_internal.push(&token_series_id);
            }
            self.mint_bundles.insert(&mint_bundle_id.clone(), &MintBundle {
                token_series_ids: Some(token_series_ids_internal),
                token_ids: None,
                price: match price {
                    Some(x) => Some(x.0),
                    None => None
                },
                limit_buy,
                bought_account_ids: LookupMap::new(StorageKey::BoughtAccountId { mint_bundle_id }),
            });
        } else if token_ids.is_some() {
            assert!(
                token_series_ids.is_none(),
                "Must chose either token_series_ids or token_ids"
            );
            panic!("Token Ids not supported for now");
        }

        refund_deposit(env::storage_usage() - initial_storage_usage, 0);

        true
    }

    #[payable]
    pub fn delete_mint_bundle(
        &mut self,
        mint_bundle_id: MintBundleId
    ) {
        assert_one_yocto();
        assert_eq!(env::predecessor_account_id(), self.tokens.owner_id, "Marble: Only owner");
        self.mint_bundles.remove(&mint_bundle_id);
    }

    #[payable]
    pub fn set_price_mint_bundle(
        &mut self,
        mint_bundle_id: MintBundleId,
        price: U128
    ) {
        assert_one_yocto();
        assert_eq!(env::predecessor_account_id(), self.tokens.owner_id, "Marble: Only owner");
        let mut mint_bundle = self.mint_bundles.get(&mint_bundle_id).unwrap();
        mint_bundle.price = Some(price.0);
        self.mint_bundles.insert(&mint_bundle_id, &mint_bundle);
    }


    // CUSTOM VIEWS

    pub fn get_buy_count_mint_bundle(
        &self,
        mint_bundle_id: MintBundleId,
        account_id: ValidAccountId
    ) -> u32 {
        let mint_bundle = self.mint_bundles.get(&mint_bundle_id).unwrap();
        mint_bundle.bought_account_ids.get(&account_id.to_string()).unwrap_or(0)
    }

    pub fn get_mint_bundle(
        &self,
        mint_bundle_id: MintBundleId
    ) -> MintBundleJson {
        let mint_bundle = self.mint_bundles.get(&mint_bundle_id).unwrap();
        MintBundleJson {
            token_series_ids: match mint_bundle.token_series_ids {
                Some(x) => Some(x.to_vec()),
                None => None
            },
            token_ids: match mint_bundle.token_ids {
                Some(x) => Some(x.to_vec()),
                None => None
            },
            price: match mint_bundle.price {
                Some(x) => Some(U128(x)),
                None => None
            },
            limit_buy: mint_bundle.limit_buy
        }
    }

    pub fn nft_get_series_single(&self, token_series_id: TokenSeriesId) -> TokenSeriesJson {
        let token_series = self.token_series_by_id.get(&token_series_id).expect("Series does not exist");
        TokenSeriesJson {
            token_series_id,
            metadata: token_series.metadata,
            creator_id: token_series.creator_id,
            royalty: token_series.royalty,
        }
    }

    pub fn nft_get_series_format(self) -> (char, &'static str, &'static str) {
        (TOKEN_DELIMETER, TITLE_DELIMETER, EDITION_DELIMETER)
    }

    pub fn nft_get_series_price(self, token_series_id: TokenSeriesId) -> Option<U128> {
        let price = self.token_series_by_id.get(&token_series_id).unwrap().price;
        match price {
            Some(p) => return Some(U128::from(p)),
            None => return None
        };
    }

    pub fn nft_get_series(
        &self,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<TokenSeriesJson> {
        let start_index: u128 = from_index.map(From::from).unwrap_or_default();
        assert!(
            (self.token_series_by_id.len() as u128) > start_index,
            "Out of bounds, please use a smaller from_index."
        );
        let limit = limit.map(|v| v as usize).unwrap_or(usize::MAX);
        assert_ne!(limit, 0, "Cannot provide limit of 0.");

        self.token_series_by_id
            .iter()
            .skip(start_index as usize)
            .take(limit)
            .map(|(token_series_id, token_series)| TokenSeriesJson {
                token_series_id,
                metadata: token_series.metadata,
                creator_id: token_series.creator_id,
                royalty: token_series.royalty,
            })
            .collect()
    }

    pub fn nft_supply_for_series(&self, token_series_id: TokenSeriesId) -> U64 {
        self.token_series_by_id.get(&token_series_id).expect("Token series not exist").tokens.len().into()
    }

    pub fn nft_tokens_by_series(
        &self,
        token_series_id: TokenSeriesId,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<Token> {
        let start_index: u128 = from_index.map(From::from).unwrap_or_default();
        let tokens = self.token_series_by_id.get(&token_series_id).unwrap().tokens;
        assert!(
            (tokens.len() as u128) > start_index,
            "Out of bounds, please use a smaller from_index."
        );
        let limit = limit.map(|v| v as usize).unwrap_or(usize::MAX);
        assert_ne!(limit, 0, "Cannot provide limit of 0.");

        tokens
            .iter()
            .skip(start_index as usize)
            .take(limit)
            .map(|token_id| self.nft_token(token_id).unwrap())
            .collect()
    }

    pub fn nft_token(&self, token_id: TokenId) -> Option<Token> {
        let owner_id = self.tokens.owner_by_id.get(&token_id)?;
        let approved_account_ids = self
            .tokens
            .approvals_by_id
            .as_ref()
            .and_then(|by_id| by_id.get(&token_id).or_else(|| Some(HashMap::new())));

        // CUSTOM (switch metadata for the token_series metadata)
        let mut token_id_iter = token_id.split(TOKEN_DELIMETER);
        let token_series_id = token_id_iter.next().unwrap().parse().unwrap();
        let series_metadata = self.token_series_by_id.get(&token_series_id).unwrap().metadata;

        let mut token_metadata = self.tokens.token_metadata_by_id.as_ref().unwrap().get(&token_id).unwrap();

        token_metadata.title = Some(format!(
            "{}{}{}",
            series_metadata.title.unwrap(),
            TITLE_DELIMETER,
            token_id_iter.next().unwrap()
        ));

        // token_metadata.reference = series_metadata.reference;
        // token_metadata.media = series_metadata.media;
        // token_metadata.copies = series_metadata.copies;
        token_metadata.extra = series_metadata.reference;

        Some(Token {
            token_id,
            owner_id,
            metadata: Some(token_metadata),
            approved_account_ids,
        })
    }

    // CUSTOM core standard repeated here because no macro below

    pub fn nft_transfer_unsafe(
        &mut self,
        receiver_id: ValidAccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    ) {
        let sender_id = env::predecessor_account_id();
        let receiver_id_str = receiver_id.to_string();
        let (previous_owner_id, _) = self.tokens.internal_transfer(&sender_id, &receiver_id_str, &token_id, approval_id, memo.clone());

        let authorized_id: Option<AccountId> = if sender_id != previous_owner_id {
            Some(sender_id)
        } else {
            None
        };

        NearEvent::log_nft_transfer(
            previous_owner_id,
            receiver_id_str,
            vec![token_id],
            memo,
            authorized_id,
        );
    }

    #[payable]
    pub fn nft_transfer(
        &mut self,
        receiver_id: ValidAccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    ) {
        let sender_id = env::predecessor_account_id();
        let previous_owner_id = self.tokens.owner_by_id.get(&token_id).expect("Token not found");
        let receiver_id_str = receiver_id.to_string();
        self.tokens.nft_transfer(receiver_id, token_id.clone(), approval_id, memo.clone());

        let authorized_id: Option<AccountId> = if sender_id != previous_owner_id {
            Some(sender_id)
        } else {
            None
        };

        NearEvent::log_nft_transfer(
            previous_owner_id,
            receiver_id_str,
            vec![token_id],
            memo,
            authorized_id,
        );
    }

    #[payable]
    pub fn nft_transfer_call(
        &mut self,
        receiver_id: ValidAccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<bool> {
        assert_one_yocto();
        let sender_id = env::predecessor_account_id();
        let (previous_owner_id, old_approvals) = self.tokens.internal_transfer(
            &sender_id,
            receiver_id.as_ref(),
            &token_id,
            approval_id,
            memo.clone(),
        );

        let authorized_id: Option<AccountId> = if sender_id != previous_owner_id {
            Some(sender_id.clone())
        } else {
            None
        };

        NearEvent::log_nft_transfer(
            previous_owner_id.clone(),
            receiver_id.to_string(),
            vec![token_id.clone()],
            memo,
            authorized_id,
        );

        // Initiating receiver's call and the callback
        ext_non_fungible_token_receiver::nft_on_transfer(
            sender_id,
            previous_owner_id.clone(),
            token_id.clone(),
            msg,
            receiver_id.as_ref(),
            NO_DEPOSIT,
            env::prepaid_gas() - GAS_FOR_NFT_TRANSFER_CALL,
        )
            .then(ext_self::nft_resolve_transfer(
                previous_owner_id,
                receiver_id.into(),
                token_id,
                old_approvals,
                &env::current_account_id(),
                NO_DEPOSIT,
                GAS_FOR_RESOLVE_TRANSFER,
            ))
            .into()
    }

    // CUSTOM enumeration standard modified here because no macro below

    pub fn nft_total_supply(&self) -> U128 {
        (self.tokens.owner_by_id.len() as u128).into()
    }

    pub fn nft_tokens(&self, from_index: Option<U128>, limit: Option<u64>) -> Vec<Token> {
        // Get starting index, whether or not it was explicitly given.
        // Defaults to 0 based on the spec:
        // https://nomicon.io/Standards/NonFungibleToken/Enumeration.html#interface
        let start_index: u128 = from_index.map(From::from).unwrap_or_default();
        assert!(
            (self.tokens.owner_by_id.len() as u128) > start_index,
            "Out of bounds, please use a smaller from_index."
        );
        let limit = limit.map(|v| v as usize).unwrap_or(usize::MAX);
        assert_ne!(limit, 0, "Cannot provide limit of 0.");
        self.tokens
            .owner_by_id
            .iter()
            .skip(start_index as usize)
            .take(limit)
            .map(|(token_id, _)| self.nft_token(token_id).unwrap())
            .collect()
    }

    pub fn nft_supply_for_owner(self, account_id: ValidAccountId) -> U128 {
        let tokens_per_owner = self.tokens.tokens_per_owner.expect(
            "Could not find tokens_per_owner when calling a method on the enumeration standard.",
        );
        tokens_per_owner
            .get(account_id.as_ref())
            .map(|account_tokens| U128::from(account_tokens.len() as u128))
            .unwrap_or(U128(0))
    }

    pub fn nft_tokens_for_owner(
        &self,
        account_id: ValidAccountId,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<Token> {
        let tokens_per_owner = self.tokens.tokens_per_owner.as_ref().expect(
            "Could not find tokens_per_owner when calling a method on the enumeration standard.",
        );
        let token_set = if let Some(token_set) = tokens_per_owner.get(account_id.as_ref()) {
            token_set
        } else {
            return vec![];
        };
        let limit = limit.map(|v| v as usize).unwrap_or(usize::MAX);
        assert_ne!(limit, 0, "Cannot provide limit of 0.");
        let start_index: u128 = from_index.map(From::from).unwrap_or_default();
        assert!(
            token_set.len() as u128 > start_index,
            "Out of bounds, please use a smaller from_index."
        );
        token_set
            .iter()
            .skip(start_index as usize)
            .take(limit)
            .map(|token_id| self.nft_token(token_id).unwrap())
            .collect()
    }

    pub fn nft_payout(
        &self,
        token_id: TokenId,
        balance: U128,
        max_len_payout: u32,
    ) -> Payout {
        let owner_id = self.tokens.owner_by_id.get(&token_id).expect("No token id");
        let mut token_id_iter = token_id.split(TOKEN_DELIMETER);
        let token_series_id = token_id_iter.next().unwrap().parse().unwrap();
        let royalty = self.token_series_by_id.get(&token_series_id).expect("no type").royalty;

        assert!(royalty.len() as u32 <= max_len_payout, "Market cannot payout to that many receivers");

        let balance_u128: u128 = balance.into();

        let mut payout: Payout = Payout { payout: HashMap::new() };
        let mut total_perpetual = 0;

        for (k, v) in royalty.iter() {
            if *k != owner_id {
                let key = k.clone();
                payout.payout.insert(key, royalty_to_payout(*v, balance_u128));
                total_perpetual += *v;
            }
        }
        payout.payout.insert(owner_id, royalty_to_payout(10000 - total_perpetual, balance_u128));
        payout
    }

    #[payable]
    pub fn nft_transfer_payout(
        &mut self,
        receiver_id: ValidAccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        balance: Option<U128>,
        max_len_payout: Option<u32>,
    ) -> Option<Payout> {
        assert_one_yocto();

        let sender_id = env::predecessor_account_id();
        // Transfer
        let previous_token = self.nft_token(token_id.clone()).expect("no token");
        self.tokens.nft_transfer(receiver_id.clone(), token_id.clone(), approval_id, None);

        // Payout calculation
        let previous_owner_id = previous_token.owner_id;
        let mut total_perpetual = 0;
        let payout = if let Some(balance) = balance {
            let balance_u128: u128 = u128::from(balance);
            let mut payout: Payout = Payout { payout: HashMap::new() };

            let mut token_id_iter = token_id.split(TOKEN_DELIMETER);
            let token_series_id = token_id_iter.next().unwrap().parse().unwrap();
            let royalty = self.token_series_by_id.get(&token_series_id).expect("no type").royalty;

            assert!(royalty.len() as u32 <= max_len_payout.unwrap(), "Market cannot payout to that many receivers");
            for (k, v) in royalty.iter() {
                let key = k.clone();
                if key != previous_owner_id {
                    payout.payout.insert(key, royalty_to_payout(*v, balance_u128));
                    total_perpetual += *v;
                }
            }

            assert!(
                total_perpetual <= 10000,
                "Total payout overflow"
            );

            payout.payout.insert(previous_owner_id.clone(), royalty_to_payout(10000 - total_perpetual, balance_u128));
            Some(payout)
        } else {
            None
        };

        let authorized_id: Option<AccountId> = if sender_id != previous_owner_id {
            Some(sender_id)
        } else {
            None
        };

        NearEvent::log_nft_transfer(
            previous_owner_id,
            receiver_id.to_string(),
            vec![token_id],
            None,
            authorized_id,
        );

        payout
    }

    pub fn get_owner(&self) -> AccountId {
        self.tokens.owner_id.clone()
    }
}

fn royalty_to_payout(a: u32, b: Balance) -> U128 {
    U128(a as u128 * b / 10_000u128)
}

// near_contract_standards::impl_non_fungible_token_core!(Contract, tokens);
// near_contract_standards::impl_non_fungible_token_enumeration!(Contract, tokens);
near_contract_standards::impl_non_fungible_token_approval!(Contract, tokens);

#[near_bindgen]
impl NonFungibleTokenMetadataProvider for Contract {
    fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }
}

#[near_bindgen]
impl NonFungibleTokenResolver for Contract {
    #[private]
    fn nft_resolve_transfer(
        &mut self,
        previous_owner_id: AccountId,
        receiver_id: AccountId,
        token_id: TokenId,
        approved_account_ids: Option<HashMap<AccountId, u64>>,
    ) -> bool {
        let resp: bool = self.tokens.nft_resolve_transfer(
            previous_owner_id.clone(),
            receiver_id.clone(),
            token_id.clone(),
            approved_account_ids,
        );

        // if not successful, return nft back to original owner
        if !resp {
            NearEvent::log_nft_transfer(
                receiver_id,
                previous_owner_id,
                vec![token_id],
                None,
                None,
            );
        }

        resp
    }
}

/// from https://github.com/near/near-sdk-rs/blob/e4abb739ff953b06d718037aa1b8ab768db17348/near-contract-standards/src/non_fungible_token/utils.rs#L29

fn refund_deposit(storage_used: u64, extra_spend: Balance) {
    let required_cost = env::storage_byte_cost() * Balance::from(storage_used);
    let attached_deposit = env::attached_deposit() - extra_spend;

    assert!(
        required_cost <= attached_deposit,
        "Must attach {} yoctoNEAR to cover storage",
        required_cost,
    );

    let refund = attached_deposit - required_cost;
    if refund > 1 {
        Promise::new(env::predecessor_account_id()).transfer(refund);
    }
}

fn get_random_number(shift_amount: u32) -> u32 {
    let mut seed = env::random_seed();
    let seed_len = seed.len();
    let mut arr: [u8; 4] = Default::default();
    seed.rotate_left(shift_amount as usize % seed_len);
    arr.copy_from_slice(&seed[..4]);
    u32::from_le_bytes(arr)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::MockedBlockchain;
    use near_sdk::{testing_env};
    use serde_with::with_prefix;

    const STORAGE_FOR_CREATE_SERIES: Balance = 8540000000000000000000;
    const STORAGE_FOR_MINT: Balance = 11280000000000000000000;

    fn get_context(predecessor_account_id: ValidAccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }

    fn setup_contract() -> (VMContextBuilder, Contract) {
        let mut context = VMContextBuilder::new();
        testing_env!(context.predecessor_account_id(accounts(0)).build());
        let contract = Contract::new_default_meta(accounts(1), accounts(4));
        (context, contract)
    }

    #[test]
    fn test_new() {
        let mut context = get_context(accounts(1));
        testing_env!(context.build());
        let contract = Contract::new(
            accounts(1),
            accounts(4),
            NFTContractMetadata {
                spec: NFT_METADATA_SPEC.to_string(),
                name: "Triple Triad".to_string(),
                symbol: "TRIAD".to_string(),
                icon: Some(DATA_IMAGE_SVG_COMIC_ICON.to_string()),
                base_uri: Some("https://ipfs.fleek.co/ipfs/".to_string()),
                reference: None,
                reference_hash: None,
            },
        );
        testing_env!(context.is_view(true).build());
        assert_eq!(contract.get_owner(), accounts(1).to_string());
        assert_eq!(contract.nft_metadata().base_uri.unwrap(), "https://ipfs.fleek.co/ipfs/".to_string());
        assert_eq!(contract.nft_metadata().icon.unwrap(), DATA_IMAGE_SVG_COMIC_ICON.to_string());
    }

    fn create_series(
        contract: &mut Contract,
        royalty: &HashMap<AccountId, u32>,
        price: Option<U128>,
        copies: Option<u64>,
    ) {
        contract.nft_create_series(
            TokenMetadata {
                title: Some("Tsundere land".to_string()),
                description: None,
                media: Some(
                    "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
                ),
                media_hash: None,
                copies: copies,
                issued_at: None,
                expires_at: None,
                starts_at: None,
                updated_at: None,
                extra: None,
                reference: Some(
                    "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
                ),
                reference_hash: None,
            },
            price,
            Some(royalty.clone()),
            accounts(1),
        );
    }

    #[test]
    fn test_create_series() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);
        create_series(
            &mut contract,
            &royalty,
            Some(U128::from(1 * 10u128.pow(24))),
            None,
        );

        let nft_series_return = contract.nft_get_series_single("1".to_string());
        assert_eq!(
            nft_series_return.creator_id,
            accounts(1).to_string()
        );

        assert_eq!(
            nft_series_return.token_series_id,
            "1",
        );

        assert_eq!(
            nft_series_return.royalty,
            royalty,
        );

        assert!(
            nft_series_return.metadata.copies.is_none()
        );

        assert_eq!(
            nft_series_return.metadata.title.unwrap(),
            "Tsundere land".to_string()
        );

        assert_eq!(
            nft_series_return.metadata.reference.unwrap(),
            "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
        );
    }

    #[test]
    fn test_buy() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);
        let metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some(
                "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
            ),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some(
                "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
            ),
            reference_hash: None,
        };
        create_series(
            &mut contract,
            &royalty,
            Some(U128::from(1 * 10u128.pow(24))),
            None,
        );

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(1 * 10u128.pow(24) + STORAGE_FOR_MINT)
            .build()
        );

        let token_id = contract.nft_buy("1".to_string(), accounts(2), Some(metadata));

        let token_from_nft_token = contract.nft_token(token_id);
        assert_eq!(
            token_from_nft_token.unwrap().owner_id,
            accounts(2).to_string()
        )
    }

    #[test]
    fn test_mint() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );
        let metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some(
                "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
            ),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some(
                "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
            ),
            reference_hash: None,
        };
        let token_id = contract.nft_mint("1".to_string(), accounts(2), None);

        let token_from_nft_token = contract.nft_token(token_id);
        assert_eq!(
            token_from_nft_token.unwrap().owner_id,
            accounts(2).to_string()
        )
    }

    #[test]
    #[should_panic(expected = "Marble: Token series is not mintable")]
    fn test_invalid_mint_non_mintable() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(1)
            .build()
        );
        contract.nft_set_series_non_mintable("1".to_string());

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        contract.nft_mint("1".to_string(), accounts(2), None);
    }

    #[test]
    #[should_panic(expected = "Marble: Token series is not mintable")]
    fn test_invalid_mint_above_copies() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(1));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );
        let metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some(
                "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
            ),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some(
                "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
            ),
            reference_hash: None,
        };
        contract.nft_mint("1".to_string(), accounts(2),Some(metadata));
        contract.nft_mint("1".to_string(), accounts(2), None);
    }

    #[test]
    fn test_decrease_copies() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(5));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        contract.nft_mint("1".to_string(), accounts(2), None);
        contract.nft_mint("1".to_string(), accounts(2), None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(1)
            .build()
        );

        contract.nft_decrease_series_copies("1".to_string(), U64::from(3));
    }

    #[test]
    #[should_panic(expected = "Marble: cannot decrease supply, already minted : 2")]
    fn test_invalid_decrease_copies() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(5));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        contract.nft_mint("1".to_string(), accounts(2), None);
        contract.nft_mint("1".to_string(), accounts(2), None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(1)
            .build()
        );

        contract.nft_decrease_series_copies("1".to_string(), U64::from(4));
    }

    #[test]
    #[should_panic(expected = "Marble: not for sale")]
    fn test_invalid_buy_price_null() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, Some(U128::from(1 * 10u128.pow(24))), None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(1)
            .build()
        );

        contract.nft_set_series_price("1".to_string(), None);

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(1 * 10u128.pow(24) + STORAGE_FOR_MINT)
            .build()
        );

        let token_id = contract.nft_buy("1".to_string(), accounts(2), None);

        let token_from_nft_token = contract.nft_token(token_id);
        assert_eq!(
            token_from_nft_token.unwrap().owner_id,
            accounts(2).to_string()
        )
    }

    #[test]
    fn test_nft_burn() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        let metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some(
                "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
            ),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some(
                "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
            ),
            reference_hash: None,
        };
        let token_id = contract.nft_mint("1".to_string(), accounts(2), Some(metadata));

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(1)
            .build()
        );

        contract.nft_burn(token_id.clone());
        let token = contract.nft_token(token_id);
        assert!(token.is_none());
    }

    #[test]
    fn test_nft_transfer() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        let metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some(
                "bafybeidzcan4nzcz7sczs4yzyxly4galgygnbjewipj6haco4kffoqpkiy".to_string()
            ),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some(
                "bafybeicg4ss7qh5odijfn2eogizuxkrdh3zlv4eftcmgnljwu7dm64uwji".to_string()
            ),
            reference_hash: None,
        };
        let token_id = contract.nft_mint("1".to_string(), accounts(2), Some(metadata));

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(1)
            .build()
        );

        contract.nft_transfer(accounts(3), token_id.clone(), None, None);

        let token = contract.nft_token(token_id).unwrap();
        assert_eq!(
            token.owner_id,
            accounts(3).to_string()
        )
    }

    #[test]
    fn test_nft_transfer_unsafe() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        let token_id = contract.nft_mint("1".to_string(), accounts(2), None);

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .build()
        );

        contract.nft_transfer_unsafe(accounts(3), token_id.clone(), None, None);

        let token = contract.nft_token(token_id).unwrap();
        assert_eq!(
            token.owner_id,
            accounts(3).to_string()
        )
    }

    #[test]
    fn test_nft_transfer_payout() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, None);

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_MINT)
            .build()
        );

        let token_id = contract.nft_mint("1".to_string(), accounts(2), None);

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(1)
            .build()
        );

        let payout = contract.nft_transfer_payout(
            accounts(3),
            token_id.clone(),
            Some(0),
            Some(U128::from(1 * 10u128.pow(24))),
            Some(10),
        );

        let mut payout_calc: HashMap<AccountId, U128> = HashMap::new();
        payout_calc.insert(
            accounts(1).to_string(),
            U128::from((1000 * (1 * 10u128.pow(24))) / 10_000),
        );
        payout_calc.insert(
            accounts(2).to_string(),
            U128::from((9000 * (1 * 10u128.pow(24))) / 10_000),
        );

        assert_eq!(payout.unwrap().payout, payout_calc);

        let token = contract.nft_token(token_id).unwrap();
        assert_eq!(
            token.owner_id,
            accounts(3).to_string()
        )
    }

    #[test]
    fn test_create_mint_bundle() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        contract.create_mint_bundle(
            "test-bundle-test".to_string(),
            Some(vec!["1".to_string(), "2".to_string()]),
            None,
            Some(U128::from(5 * 10u128.pow(24))),
            None
        );

        let mint_bundle = contract.get_mint_bundle("test-bundle-test".to_string());

        assert_eq!(mint_bundle.token_series_ids, Some(vec!["1".to_string(), "2".to_string()]));
        assert_eq!(mint_bundle.token_ids, None);
        assert_eq!(mint_bundle.price.unwrap(), U128::from(5 * 10u128.pow(24)));
        assert_eq!(mint_bundle.limit_buy, None);
    }

    #[test]
    fn test_buy_mint_bundle() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(2));
        create_series(&mut contract, &royalty, None, Some(2));
        create_series(&mut contract, &royalty, None, Some(2));
        create_series(&mut contract, &royalty, None, Some(2));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let price = 5 * 10u128.pow(24);
        let mint_bundle_id = "test-bundle-test".to_string();
        contract.create_mint_bundle(
            mint_bundle_id.clone(),
            Some(vec!["1".to_string(), "2".to_string(), "3".to_string(), "4".to_string()]),
            None,
            Some(U128::from(price)),
            None
        );

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(price + STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id, accounts(2));
    }

    #[test]
    #[should_panic(expected = "Marble: Mint bundle does not exist or already finished")]
    fn test_invalid_exhaust_mint_bundle() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(2));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let price = 5 * 10u128.pow(24);
        let mint_bundle_id = "test-bundle-test".to_string();
        contract.create_mint_bundle(
            mint_bundle_id.clone(),
            Some(vec!["1".to_string()]),
            None,
            Some(U128::from(price)),
            None
        );

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(price + STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id, accounts(2));
    }

    #[test]
    #[should_panic(expected = "Marble: Mint exhausted for account_id charlie")]
    fn test_invalid_exhaust_limit_buy_mint_bundle() {
        let (mut context, mut contract) = setup_contract();
        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let mut royalty: HashMap<AccountId, u32> = HashMap::new();
        royalty.insert(accounts(1).to_string(), 1000);

        create_series(&mut contract, &royalty, None, Some(2));

        testing_env!(context
            .predecessor_account_id(accounts(1))
            .attached_deposit(STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        let price = 5 * 10u128.pow(24);
        let mint_bundle_id = "test-bundle-test".to_string();
        contract.create_mint_bundle(
            mint_bundle_id.clone(),
            Some(vec!["1".to_string()]),
            None,
            Some(U128::from(price)),
            Some(1)
        );

        testing_env!(context
            .predecessor_account_id(accounts(2))
            .attached_deposit(price + STORAGE_FOR_CREATE_SERIES)
            .build()
        );

        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id.clone(), accounts(2));
        contract.buy_mint_bundle(mint_bundle_id, accounts(2));
    }
}