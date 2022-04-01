use bitcoin::util::amount::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::template::Template;
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::timelocks::AbsHeight;
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
// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
use std::sync::Arc;

/// # Dutch Auction Data
/// Additional information required to initiate a dutch auction
#[derive(JsonSchema, Serialize, Deserialize)]
struct DutchAuctionData {
    /// How often should we decreate the price, in blocks
    period: u16,
    /// what price should we start at?
    start_price: AmountU64,
    /// what price should we stop at?
    min_price: AmountU64,
    /// how many price decreases should we do?
    updates: u64,
}

impl DutchAuctionData {
    /// # Create a Schedule for Sale
    /// computes, based on a start time, the list of heights and prices
    fn create_schedule(
        &self,
        start_height: AbsHeight,
    ) -> Result<Vec<(AbsHeight, AmountU64)>, CompilationError> {
        let mut start: Amount = self.start_price.into();
        let stop: Amount = self.min_price.into();
        let inc = (start - stop) / self.updates;
        let mut h: u32 = start_height.get();
        let mut sched = vec![(start_height, self.start_price)];
        for _ in 1..self.updates {
            h += self.period as u32;
            start -= inc;
            sched.push((AbsHeight::try_from(h)?, start.into()));
        }
        Ok(sched)
    }
    /// derives a default auction where the price drops every 6
    /// blocks (1 time per hour), from 10x to 1x the sale price specified,
    /// spanning a month of blocks.
    fn derive_default(main: &NFT_Sale_Trait_Version_0_1_0) -> Self {
        DutchAuctionData {
            // every 6 blocks
            period: 6,
            start_price: (Amount::from(main.price) * 10u64).into(),
            min_price: main.price,
            // 144 blocks/day
            updates: 144 * 30 / 6,
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct NFTDutchAuction {
    /// This data can be specified directly, or default derived from main
    extra: DutchAuctionData,
    /// The main trait data
    main: NFT_Sale_Trait_Version_0_1_0,
}

/// # Versions Trait Wrapper
#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    /// Use the Actual Trait API
    NFT_Sale_Trait_Version_0_1_0(NFT_Sale_Trait_Version_0_1_0),
    /// Directly Specify the Data
    Exact(DutchAuctionData, NFT_Sale_Trait_Version_0_1_0),
}
impl Contract for NFTDutchAuction {
    declare! {updatable<()>, Self::transfer}
}
fn default_coerce<T>(_: T) -> Result<(), CompilationError> {
    Ok(())
}
impl TryFrom<Versions> for NFTDutchAuction {
    type Error = CompilationError;
    fn try_from(v: Versions) -> Result<NFTDutchAuction, Self::Error> {
        Ok(match v {
            Versions::NFT_Sale_Trait_Version_0_1_0(main) => {
                // attempt to get the data from the JSON:
                // - if extra data, must deserialize
                //   - return any errors?
                // - if no extra data, derive.
                let extra = main
                    .extra
                    .clone()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|_| CompilationError::TerminateCompilation)?
                    .unwrap_or_else(|| DutchAuctionData::derive_default(&main));
                NFTDutchAuction { main, extra }
            }
            Versions::Exact(extra, main) => {
                if extra.start_price < extra.min_price || extra.period == 0 || extra.updates == 0 {
                    // Nonsense
                    return Err(CompilationError::TerminateCompilation);
                }
                NFTDutchAuction { main, extra }
            }
        })
    }
}

REGISTER![[NFTDutchAuction, Versions], "logo.png"];

impl NFTDutchAuction {
    /// # signed
    /// sales must be signed by the current owner
    #[guard]
    fn signed(self, ctx: Context) {
        Clause::Key(self.main.data.owner.clone())
    }
    /// # transfer
    /// transfer exchanges the NFT for cold hard Bitcoinz
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn transfer(self, base_ctx: Context, u: ()) {
        let mut ret = vec![];
        let schedule = self.extra.create_schedule(self.main.sale_time)?;
        let mut base_ctx = base_ctx;
        // the main difference is we iterate over the schedule here
        for (nth, sched) in schedule.iter().enumerate() {
            let mut ctx = base_ctx.derive_num(nth as u64)?;
            let amt = ctx.funds();
            // first, let's get the module that should be used to 're-mint' this NFT
            // to the new owner
            let key = self
                .main
                .data
                .minting_module
                .clone()
                .ok_or(CompilationError::TerminateCompilation)?
                .key;
            // let's make a copy of the old nft metadata..
            let mut mint_data = self.main.data.clone();
            // and change the owner to the buyer
            mint_data.owner = self.main.sell_to;
            let new_ctx = ctx.derive_str(Arc::new("transfer".into()))?;
            // let's now compile a new 'mint' of the NFT
            let create_args = CreateArgs {
                context: ContextualArguments {
                    amount: ctx.funds(),
                    network: ctx.network,
                    effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                },
                arguments: mint_impl::Versions::Mint_NFT_Trait_Version_0_1_0(mint_data),
            };
            let new_nft_contract = create_contract_by_key(new_ctx, &key, create_args)
                .map_err(|_| CompilationError::TerminateCompilation)?;
            // Now for the magic:
            // This is a transaction that creates at output 0 the new nft for the
            // person, and must add another input that pays sufficiently to pay the
            // prior owner an amount.

            // todo: we also could use cut-through here once implemented
            // todo: change seem problematic here? with a bit of work, we could handle it
            // cleanly if the buyer identifys an output they are spending before requesting
            // a purchase.
            let price: Amount = sched.1.into();
            let mut tmpl = ctx
                .template()
                .add_output(amt, &new_nft_contract, None)?
                .add_amount(price)
                .add_sequence()
                // only active at the set time
                .set_lock_time(sched.0.into())?;
            let t = if let Some(artist) = self.main.data.ipfs_nft.artist {
                // Pay Sale to Seller
                tmpl.add_output(
                    Amount::from_btc(price.as_btc() * (1.0 - self.main.data.royalty))?,
                    &self.main.data.owner,
                    None,
                )?
                // Pay Royalty to Creator
                .add_output(
                    Amount::from_btc(price.as_btc() as f64 * self.main.data.royalty)?,
                    &artist,
                    None,
                )?
            } else {
                // Pay Sale to Seller
                tmpl.add_output(
                    Amount::from_btc(price.as_btc())?,
                    &self.main.data.owner,
                    None,
                )?
            };
            ret.push(Ok(t.into()));
        }
        Ok(Box::new(ret.into_iter()))
    }
}
