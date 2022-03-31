// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use bitcoin::util::amount::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_nft_trait::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
#[deny(missing_docs)]
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
    declare! {updatable<()>, Self::transfer}
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
    /// # signed
    /// sales must be signed by the current owner
    #[guard]
    fn signed(self, ctx: Context) {
        Clause::Key(self.0.data.owner.clone())
    }
    /// # transfer
    /// transfer exchanges the NFT for cold hard Bitcoinz
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn transfer(self, mut ctx: Context, u: ()) {
        let amt = ctx.funds();
        // first, let's get the module that should be used to 're-mint' this NFT
        // to the new owner
        let key = self
            .0
            .data
            .minting_module
            .clone()
            .ok_or(CompilationError::TerminateCompilation)?
            .key;
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
        let new_nft_contract = create_contract_by_key(new_ctx, &key, new_nft_args)
            .map_err(|_| CompilationError::TerminateCompilation)?;
        // Now for the magic:
        // This is a transaction that creates at output 0 the new nft for the
        // person, and must add another input that pays sufficiently to pay the
        // prior owner an amount.

        // todo: we also could use cut-through here once implemented
        // todo: change seem problematic here? with a bit of work, we could handle it
        // cleanly if the buyer identifys an output they are spending before requesting
        // a purchase.
        ctx.template()
            .add_output(amt, &new_nft_contract, None)?
            .add_amount(self.0.price.into())
            .add_sequence()
            .add_output(self.0.price.into(), &self.0.data.owner, None)?
            // note: what would happen if we had another output that
            // had a percentage-of-sale royalty to some creator's key?
            .into()
    }
}
