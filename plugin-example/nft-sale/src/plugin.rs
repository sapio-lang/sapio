// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![deny(missing_docs)]

//! NFT Sale Contract

use bitcoin::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::*;
use sapio_wasm_nft_trait::*;
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
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.0.data.validate()?;
        ctx.funds()
            .checked_add(self.0.price.into())
            .ok_or(CompilationError::OutOfFunds)?;
        Ok(ctx.funds())
    }
}
impl From<Versions> for SimpleNFTSale {
    fn from(v: Versions) -> SimpleNFTSale {
        let Versions::NFT_Sale_Trait_Version_0_1_0(x) = v;
        SimpleNFTSale(x)
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![[SimpleNFTSale, Versions], "logo.png"];

impl SimpleNFTSale {
    /// # transfer
    /// transfer exchanges the NFT for cold hard Bitcoinz
    #[then]
    fn transfer(self, mut ctx: Context) {
        self.ensure_amount(ctx.derive_str(Arc::new("validate".into()))?)?;
        let amt = ctx.funds();
        // first, let's get the module that should be used to 're-mint' this NFT
        // to the new owner
        let mut minting_module = self
            .0
            .data
            .minting_module
            .as_ref()
            .ok_or_else(|| CompilationError::TerminateWith("Must Provide Module Hash".into()))?
            .clone();
        // let's make a copy of the old nft metadata..
        let mut mint_data = self.0.data.clone();
        // and change the owner to the buyer
        mint_data.owner = self.0.sell_to;
        let new_ctx = ctx.derive_str(Arc::new("transfer".into()))?;
        // let's now compile a new 'mint' of the NFT
        let new_nft_args = CreateArgs {
            context: ContextualArguments {
                lowering: ctx.lowering_plan().clone(),
                amount: ctx.funds(),
                network: ctx.network,
                effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                ordinals_info: ctx.get_ordinals().clone(),
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
        let price: Amount = self.0.price.into();
        let mut template = ctx
            .template()
            .add_output(amt, &new_nft_contract, None)?
            .set_lock_time(self.0.sale_time.into())?;
        if price != Amount::ZERO {
            template = template.add_sequence().add_amount(price)?;
        }
        let artist_gets = if self.0.data.ipfs_nft.artist.is_some() {
            self.0.data.compute_royalty_for_artist(price)?
        } else {
            Amount::ZERO
        };
        let seller_gets = price - artist_gets;
        if seller_gets != Amount::ZERO {
            template = template.add_output(seller_gets, &self.0.data.owner, None)?;
        }
        if let Some(artist) = self
            .0
            .data
            .ipfs_nft
            .artist
            .filter(|_| artist_gets != Amount::ZERO)
        {
            template = template.add_output(artist_gets, &artist, None)?;
        }
        template.into()
    }
}
