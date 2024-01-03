use bitcoin::Amount;
use sapio::contract::Contract;
use sapio::contract::StatefulArgumentsTrait;
use sapio::decl_continuation;
use sapio::util::amountrange::AmountU64;
use sapio_base::timelocks::AbsHeight;
use sapio_trait::SapioJSONTrait;
use sapio_wasm_plugin::client::*;
use schemars::*;
use serde::*;
use serde_json::Value;
pub use simp_pack::IpfsNFT;
use simp_pack::URL;
use std::convert::TryFrom;
use std::str::FromStr;
/// # Trait for a Mintable NFT
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct Mint_NFT_Trait_Version_0_1_0 {
    /// # Initial Owner
    /// The key that will own this NFT
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    pub owner: bitcoin::XOnlyPublicKey,
    /// # IPFS Sapio Interactive Metadata Protocol
    /// The Data for the NFT
    pub ipfs_nft: IpfsNFT,
    /// # Minting Module
    /// If a specific sub-module is to be used / known -- when in doubt, should
    /// be None.
    pub minting_module: Option<NFTMintingModule>,
    /// how much royalty, should be paid, as a fraction of sale (0.0 to 1.0)
    pub royalty: f64,
}

const PRECISION: u64 = 1000000;
impl Mint_NFT_Trait_Version_0_1_0 {
    pub fn compute_royalty_for_artist(&self, amount: Amount) -> Amount {
        (amount * (PRECISION as f64 * self.royalty).round() as u64) / PRECISION
    }
}

pub type NFTMintingModule = ContractModule<mint_impl::Versions>;
pub type NFTSaleModule = ContractModule<sale_impl::Versions>;

/// Boilerplate for the Mint trait
pub mod mint_impl {
    use super::*;
    #[derive(Serialize, Deserialize, JsonSchema, Clone)]
    pub enum Versions {
        Mint_NFT_Trait_Version_0_1_0(Mint_NFT_Trait_Version_0_1_0),
    }
    impl Mint_NFT_Trait_Version_0_1_0 {
        pub(crate) fn get_example() -> Self {
            let key = "9c7ad3670650f427bedac55f9a3f6779c1e7a26ab7715299aa0eadb1a09c0e62";
            let ipfs_hash = "bafkreig7r2tdlwqxzlwnd7aqhkkvzjqv53oyrkfnhksijkvmc6k57uqk6a";
            Mint_NFT_Trait_Version_0_1_0 {
                owner: bitcoin::XOnlyPublicKey::from_str(key).unwrap(),
                ipfs_nft: IpfsNFT {
                    version: 0,
                    artist: Some(bitcoin::XOnlyPublicKey::from_str(key).unwrap()),
                    cid: ipfs_hash.into(),
                    blessing: Some({
                        bitcoin::secp256k1::schnorr::Signature::from_slice(&[34; 64]).unwrap()
                    }),
                    edition: 1,
                    of_edition_count: 1,
                    softlink: Some(URL {
                        url: "https://rubin.io".into(),
                    }),
                },
                minting_module: None,
                royalty: 0.02,
            }
        }
    }
    /// we must provide an example!
    impl SapioJSONTrait for mint_impl::Versions {
        fn get_example_for_api_checking() -> Value {
            serde_json::to_value(Versions::Mint_NFT_Trait_Version_0_1_0(
                Mint_NFT_Trait_Version_0_1_0::get_example(),
            ))
            .unwrap()
        }
    }
}

/// # NFT Sale Trait
/// A trait for coordinating a sale of an NFT
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct NFT_Sale_Trait_Version_0_1_0 {
    /// # Owner
    /// The key that will own this NFT
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    pub sell_to: bitcoin::XOnlyPublicKey,
    /// # Price
    /// The price in Sats
    pub price: AmountU64,
    /// # NFT
    /// The NFT's Current Info
    pub data: Mint_NFT_Trait_Version_0_1_0,
    /// # Sale Time
    /// When the sale should be possible after
    pub sale_time: AbsHeight,
    /// # Extra Information
    /// Extra information required by this contract, if any.
    /// Optional for consumer or typechecking will fail, just pass `null`.
    /// Usually null unless you know better!
    pub extra: Option<String>,
}

/// Boilerplate for the Sale trait
pub mod sale_impl {
    use super::*;
    #[derive(Serialize, Deserialize, JsonSchema, Clone)]
    pub enum Versions {
        /// # Batching Trait API
        NFT_Sale_Trait_Version_0_1_0(NFT_Sale_Trait_Version_0_1_0),
    }
    impl SapioJSONTrait for sale_impl::Versions {
        fn get_example_for_api_checking() -> Value {
            let key = "9c7ad3670650f427bedac55f9a3f6779c1e7a26ab7715299aa0eadb1a09c0e62";
            let _ipfs_hash = "bafkreig7r2tdlwqxzlwnd7aqhkkvzjqv53oyrkfnhksijkvmc6k57uqk6a";
            serde_json::to_value(sale_impl::Versions::NFT_Sale_Trait_Version_0_1_0(
                NFT_Sale_Trait_Version_0_1_0 {
                    sell_to: bitcoin::XOnlyPublicKey::from_str(key).unwrap(),
                    price: AmountU64::from(0u64),
                    data: Mint_NFT_Trait_Version_0_1_0::get_example(),
                    sale_time: AbsHeight::try_from(0).unwrap(),
                    extra: None,
                },
            ))
            .unwrap()
        }
    }
}

/// # Sellable NFT Function
/// If a NFT should be sellable, it should have this trait implemented.
pub trait SellableNFT: Contract {
    decl_continuation! {<web={}> sell<Sell>}
}

/// # NFT Sale Trait
/// A trait for coordinating a sale of an NFT
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct NFT_Sale_Trait_Version_0_1_0_Partial {
    /// # Owner
    /// The key that will own this NFT
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    pub sell_to: bitcoin::XOnlyPublicKey,
    /// # Price
    /// The price in Sats
    pub price: AmountU64,
    /// # Sale Time
    /// When the sale should be possible after
    pub sale_time: AbsHeight,
    /// # Extra Information
    /// Extra information required by this contract, if any.
    /// Optional for consumer or typechecking will fail, just pass `null`.
    /// Usually null unless you know better!
    pub extra: Option<String>,
}

impl NFT_Sale_Trait_Version_0_1_0_Partial {
    pub fn fill(self, data: Mint_NFT_Trait_Version_0_1_0) -> NFT_Sale_Trait_Version_0_1_0 {
        NFT_Sale_Trait_Version_0_1_0 {
            data,
            sell_to: self.sell_to,
            price: self.price,
            sale_time: self.sale_time,
            extra: self.extra,
        }
    }
}

/// # Sell Instructions
#[derive(Serialize, Deserialize, JsonSchema)]
pub enum Sell {
    /// # Hold
    /// Don't transfer this NFT
    Hold,
    /// # MakeSale
    /// Transfer this NFT
    MakeSale {
        /// # Which Sale Contract to use?
        /// Specify a hash/name for a contract to generate the sale with.
        which_sale: NFTSaleModule,
        /// # The information needed to create the sale
        sale_info_partial: NFT_Sale_Trait_Version_0_1_0_Partial,
    },
}
impl Default for Sell {
    fn default() -> Sell {
        Sell::Hold
    }
}
impl StatefulArgumentsTrait for Sell {}
