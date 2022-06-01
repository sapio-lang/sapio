// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[deny(missing_docs)]
use bitcoin::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::*;
use sapio_wasm_nft_trait::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::sync::Arc;
/// # Simple NFT Sale
/// A Sale which simply transfers the NFT for a fixed price.
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct SimpleNFTSale(NFT_Sale_Trait_Version_0_1_0);

/// # Versions Trait Wrapper
#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    /// # Batching Trait API
    NFT_Sale_Trait_Version_0_1_0(NFT_Sale_Trait_Version_0_1_0),
}
impl Contract for SimpleNFTSale {
    declare! {then, Self::transfer}
    declare! {non updatable}
}
fn default_coerce<T>(_: T) -> Result<(), CompilationError> {
    Ok(())
}
impl From<Versions> for SimpleNFTSale {
    fn from(v: Versions) -> SimpleNFTSale {
        let Versions::NFT_Sale_Trait_Version_0_1_0(x) = v;
        SimpleNFTSale(x)
    }
}

REGISTER![[SimpleNFTSale, Versions], "logo.png"];

impl SimpleNFTSale {
    /// # transfer
    /// transfer exchanges the NFT for cold hard Bitcoinz
    #[then]
    fn transfer(self, mut ctx: Context) {
        let amt = ctx.funds();
        // first, let's get the module that should be used to 're-mint' this NFT
        // to the new owner
        let minting_module =
            self.0.data.minting_module.as_ref().ok_or_else(|| {
                CompilationError::TerminateWith("Must Provide Module Hash".into())
            })?;
        // let's make a copy of the old nft metadata..
        let mut mint_data = self.0.data.clone();
        // and change the owner to the buyer
        mint_data.owner = self.0.sell_to;
        let new_ctx = ctx.derive_str(Arc::new("transfer".into()))?;
        // let's now compile a new 'mint' of the NFT
        let new_nft_args = CreateArgs {
            context: ContextualArguments {
                amount: ctx.funds(),
                network: ctx.network,
                effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
            },
            arguments: mint_impl::Versions::Mint_NFT_Trait_Version_0_1_0(mint_data),
        };
        let new_nft_contract = minting_module.call(new_ctx.path(), &new_nft_args)?;
        // Now for the magic:
        // This is a transaction that creates at output 0 the new nft for the
        // person, and must add another input that pays sufficiently to pay the
        // prior owner an amount.

        // todo: we also could use cut-through here once implemented
        // todo: change seem problematic here? with a bit of work, we could handle it
        // cleanly if the buyer identifys an output they are spending before requesting
        // a purchase.
        if let Some(artist) = self.0.data.ipfs_nft.artist {
            let price: Amount = self.0.price.into();
            let artist_gets = self.0.data.compute_royalty_for_artist(price);
            let seller_gets = price - artist_gets;
            ctx.template()
                .add_amount(self.0.price.into())
                .add_output(amt, &new_nft_contract, None)?
                .add_sequence()
                .add_output(seller_gets, &self.0.data.owner, None)?
                // Pay Royalty to Creator
                .add_output(artist_gets, &artist, None)?
                // note: what would happen if we had another output that
                // had a percentage-of-sale royalty to some creator's key?
                .into()
        } else {
            ctx.template()
                .add_output(amt, &new_nft_contract, None)?
                .add_amount(self.0.price.into())
                .add_sequence()
                .add_output(self.0.price.into(), &self.0.data.owner, None)?
                .into()
        }
    }
}
