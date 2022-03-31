// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#[deny(missing_docs)]
use bitcoin::hashes::sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::amount::Amount;
use sapio::contract::empty;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
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
use std::convert::TryInto;

/// # SimpleNFT
/// A really simple NFT... not much too it!
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct SimpleNFT {
    /// The minting data, and nothing else.
    data: Mint_NFT_Trait_Version_0_1_0,
}

/// # The SimpleNFT Contract
impl Contract for SimpleNFT {
    // NFTs... only good for selling?
    declare! {updatable<Sell>, Self::sell}
    // embeds metadata
    declare! {then, Self::metadata_txns}
}

impl SimpleNFT {
    /// # unspendable
    /// what? This is just a sneaky way of making a provably unspendable branch
    /// (since the preimage of [0u8; 32] hash can never be found). We use that to
    /// help us embed metadata inside of our contract...
    #[guard]
    fn unspendable(self, ctx: Context) {
        Clause::Sha256(sha256::Hash::from_inner([0u8; 32]))
    }
    /// # Metadata TXNs
    /// This metadata TXN is provably unspendable because it is guarded
    /// by `Self::unspendable`. Neat!
    /// Here, we simple embed a OP_RETURN.
    /// But you could imagine tracking (& client side validating)
    /// an entire tree of transactions based on state transitions with these
    /// transactions... in a future post, we'll see more!
    #[then(guarded_by = "[Self::unspendable]")]
    fn metadata_txns(self, ctx: Context) {
        ctx.template()
            .add_output(
                Amount::ZERO,
                &Compiled::from_op_return(
                    &sha256::Hash::hash(&self.data.locator.as_bytes()).as_inner()[..],
                )?,
                None,
            )?
            .into()
    }
    /// # signed
    /// Get the current owners signature.
    #[guard]
    fn signed(self, ctx: Context) {
        Clause::Key(self.data.owner.clone())
    }
}
fn default_coerce(k: <SimpleNFT as Contract>::StatefulArguments) -> Result<Sell, CompilationError> {
    Ok(k)
}

impl SellableNFT for SimpleNFT {
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn sell(self, ctx: Context, sale: Sell) {
        if let Sell::MakeSale {
            sale_info,
            which_sale,
        } = sale
        {
            // if we're selling...
            if sale_info.data.owner != self.data.owner {
                // Hmmm... metadata mismatch! the current owner does not
                // matched the sale's claimed owner.
                return Err(CompilationError::TerminateCompilation);
            }
            // create a contract from the sale API passed in
            let compiled = Ok(CreateArgs {
                context: ContextualArguments {
                    amount: ctx.funds(),
                    network: ctx.network,
                    effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                },
                arguments: sale_impl::Versions::NFT_Sale_Trait_Version_0_1_0(sale_info.clone()),
            })
            .and_then(serde_json::to_value)
            // use the sale API we passed in
            .map(|args| create_contract_by_key(&which_sale.key, args, Amount::from_sat(0)))
            // handle errors...
            .map_err(|_| CompilationError::TerminateCompilation)?
            .ok_or(CompilationError::TerminateCompilation)?;
            // send to this sale!
            let mut builder = ctx.template();
            // todo: we need to cut-through the compiled contract address, but this
            // upgrade to Sapio semantics will come Soon™.
            builder = builder.add_output(compiled.amount_range.max(), &compiled, None)?;
            builder.into()
        } else {
            /// Don't do anything if we're holding!
            empty()
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    Mint_NFT_Trait_Version_0_1_0(Mint_NFT_Trait_Version_0_1_0),
}

impl TryFrom<Versions> for SimpleNFT {
    type Error = CompilationError;
    fn try_from(v: Versions) -> Result<Self, Self::Error> {
        let Versions::Mint_NFT_Trait_Version_0_1_0(mut data) = v;
        let this = LookupFrom::This
            .try_into()
            .map_err(|_| CompilationError::TerminateCompilation)?;
        match data.minting_module {
            // if no module is provided, it must be this module!
            None => {
                data.minting_module = Some(this);
                Ok(SimpleNFT { data })
            }
            // if a module is provided, we have no idea what to do...
            // unless the module is this module itself!
            Some(ref module) if module.key == this.key => Ok(SimpleNFT { data }),
            _ => Err(CompilationError::TerminateCompilation),
        }
    }
}
REGISTER![[SimpleNFT, Versions], "logo.png"];