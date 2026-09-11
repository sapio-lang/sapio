use bitcoin::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::contract::StatefulArgumentsTrait;
use sapio::decl_continuation;
use sapio::util::amountrange::AmountU64;
use sapio_base::timelocks::AbsHeight;
use sapio_wasm_plugin::client::*;
use schemars::*;
use serde::*;
pub use simp_pack::IpfsNFT;
/// # Trait for a Mintable NFT
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct Mint_NFT_Trait_Version_0_1_0 {
    /// # Initial Owner
    /// The key that will own this NFT
    pub owner: bitcoin::XOnlyPublicKey,
    /// # IPFS Sapio Interactive Metadata Protocol
    /// The Data for the NFT
    pub ipfs_nft: IpfsNFT,
    /// # Minting Module
    /// If a specific sub-module is to be used / known -- when in doubt, should
    /// be None.
    pub minting_module: Option<NFTMintingModule>,
    /// how much royalty, should be paid, as a fraction of sale (0.0 to 1.0)
    #[schemars(range(min = 0.0, max = 1.0))]
    pub royalty: f64,
}

const PRECISION: u64 = 1000000;
impl Mint_NFT_Trait_Version_0_1_0 {
    /// Validate the mint metadata before constructing a contract or sale.
    pub fn validate(&self) -> Result<(), CompilationError> {
        if !self.royalty.is_finite() || !(0.0..=1.0).contains(&self.royalty) {
            return Err(CompilationError::Custom(
                "Royalty must be a finite fraction from 0 to 1".into(),
            ));
        }
        if self.ipfs_nft.version != 0
            || self.ipfs_nft.edition == 0
            || self.ipfs_nft.edition > self.ipfs_nft.of_edition_count
        {
            return Err(CompilationError::Custom(
                "NFT metadata requires version 0 and a valid edition".into(),
            ));
        }
        Ok(())
    }

    /// Round the royalty fraction to millionths, then round the payout down to sats.
    pub fn compute_royalty_for_artist(&self, amount: Amount) -> Result<Amount, CompilationError> {
        self.validate()?;
        let fraction = (PRECISION as f64 * self.royalty).round() as u64;
        let sats = u128::from(amount.to_sat()) * u128::from(fraction) / u128::from(PRECISION);
        Ok(Amount::from_sat(sats as u64))
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
}

/// # NFT Sale Trait
/// A trait for coordinating a sale of an NFT
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct NFT_Sale_Trait_Version_0_1_0 {
    /// # Owner
    /// The key that will own this NFT
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
    /// Optional module-specific instructions interpreted by the receiving module.
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
    pub sell_to: bitcoin::XOnlyPublicKey,
    /// # Price
    /// The price in Sats
    pub price: AmountU64,
    /// # Sale Time
    /// When the sale should be possible after
    pub sale_time: AbsHeight,
    /// # Extra Information
    /// Optional module-specific instructions interpreted by the receiving module.
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

#[cfg(test)]
mod tests;
