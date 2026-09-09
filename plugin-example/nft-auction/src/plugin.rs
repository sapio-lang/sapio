// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.
#![deny(missing_docs)]

//! NFT Auction

use bitcoin::util::amount::Amount;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::timelocks::AbsHeight;
use sapio_base::Clause;
use sapio_wasm_nft_trait::*;
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;

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
    #[schemars(range(min = 1, max = 720))]
    updates: u64,
}

impl DutchAuctionData {
    /// # Create a Schedule for Sale
    /// computes, based on a start time, the list of heights and prices
    fn create_schedule(
        &self,
        start_height: AbsHeight,
    ) -> Result<Vec<(AbsHeight, AmountU64)>, CompilationError> {
        self.validate(start_height)?;
        let start = u64::from(self.start_price);
        let stop = u64::from(self.min_price);
        (0..=self.updates)
            .map(|step| {
                let height = start_height.get() as u64 + step * u64::from(self.period);
                let reduction =
                    u128::from(start - stop) * u128::from(step) / u128::from(self.updates);
                Ok((
                    AbsHeight::try_from(height as u32)?,
                    (start - reduction as u64).into(),
                ))
            })
            .collect()
    }

    fn validate(&self, start_height: AbsHeight) -> Result<(), CompilationError> {
        if self.period == 0
            || self.updates == 0
            || self.updates > 720
            || self.start_price < self.min_price
        {
            return Err(CompilationError::Custom(
                "Auction requires period > 0, 1..=720 decreases, and start >= minimum".into(),
            ));
        }
        let end = u64::from(start_height.get()) + u64::from(self.period) * self.updates;
        let end = u32::try_from(end)
            .map_err(|_| CompilationError::Custom("Auction height overflows".into()))?;
        AbsHeight::try_from(end)?;
        Ok(())
    }
    /// derives a default auction where the price drops every 6
    /// blocks (1 time per hour), from 10x to 1x the sale price specified,
    /// spanning a month of blocks.
    fn derive_default(main: &NFT_Sale_Trait_Version_0_1_0) -> Result<Self, CompilationError> {
        Ok(DutchAuctionData {
            // every 6 blocks
            period: 6,
            start_price: Amount::from(main.price)
                .checked_mul(10)
                .ok_or(CompilationError::OutOfFunds)?
                .into(),
            min_price: main.price,
            // 144 blocks/day
            updates: 144 * 30 / 6,
        })
    }
}

/// NFT Dutch Auction Contract
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
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.main.data.validate()?;
        self.extra.validate(self.main.sale_time)?;
        ctx.funds()
            .checked_add(self.extra.start_price.into())
            .ok_or(CompilationError::OutOfFunds)?;
        Ok(ctx.funds())
    }
}
fn default_coerce<T>(_: T) -> Result<(), CompilationError> {
    Ok(())
}
impl TryFrom<Versions> for NFTDutchAuction {
    type Error = CompilationError;
    fn try_from(v: Versions) -> Result<NFTDutchAuction, Self::Error> {
        let auction = match v {
            Versions::NFT_Sale_Trait_Version_0_1_0(main) => {
                // attempt to get the data from the JSON:
                // - if extra data, must deserialize
                //   - return any errors?
                // - if no extra data, derive.
                let extra = match main.extra.clone() {
                    None => DutchAuctionData::derive_default(&main)?,
                    Some(extra) => serde_json::from_str(&extra)
                        .map_err(CompilationError::DeserializationError)?,
                };
                NFTDutchAuction { main, extra }
            }
            Versions::Exact(extra, main) => NFTDutchAuction { main, extra },
        };
        auction.main.data.validate()?;
        auction.extra.validate(auction.main.sale_time)?;
        Ok(auction)
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![[NFTDutchAuction, Versions], "logo.png"];

impl NFTDutchAuction {
    /// # signed
    /// sales must be signed by the current owner
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(self.main.data.owner.clone())
    }
    /// # transfer
    /// transfer exchanges the NFT for cold hard Bitcoinz
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn transfer(self, base_ctx: Context, _u: ()) {
        let mut ret = vec![];
        let schedule = self.extra.create_schedule(self.main.sale_time)?;
        let mut base_ctx = base_ctx;
        self.ensure_amount(base_ctx.derive_str(Arc::new("validate".into()))?)?;
        let amt = base_ctx.funds();
        let mut minting_module = self
            .main
            .data
            .minting_module
            .clone()
            .ok_or_else(|| CompilationError::Custom("Must provide minting module".into()))?;
        let mut mint_data = self.main.data.clone();
        mint_data.owner = self.main.sell_to;
        let new_ctx = base_ctx.derive_str(Arc::new("transfer".into()))?;
        let create_args = CreateArgs {
            context: ContextualArguments {
                lowering: base_ctx.lowering_plan().clone(),
                amount: amt,
                network: base_ctx.network,
                effects: unsafe { base_ctx.get_effects_internal() }.as_ref().clone(),
                ordinals_info: base_ctx.get_ordinals().clone(),
            },
            arguments: mint_impl::Versions::Mint_NFT_Trait_Version_0_1_0(mint_data),
        };
        // Every price transfers the same NFT. Compile its destination once so
        // the schedule does not consume one nested module call per price.
        let new_nft_contract = minting_module.call(new_ctx.path(), &create_args)?;
        for (nth, sched) in schedule.iter().enumerate() {
            let ctx = base_ctx.derive_num(nth as u64)?;
            // Now for the magic:
            // This is a transaction that creates at output 0 the new nft for the
            // person, and must add another input that pays sufficiently to pay the
            // prior owner an amount.

            // todo: we also could use cut-through here once implemented
            // todo: change seem problematic here? with a bit of work, we could handle it
            // cleanly if the buyer identifys an output they are spending before requesting
            // a purchase.
            let price: Amount = sched.1.into();
            let mut template = ctx
                .template()
                .add_output(amt, &new_nft_contract, None)?
                .set_lock_time(sched.0.into())?;
            if price != Amount::ZERO {
                template = template.add_sequence().add_amount(price)?;
            }
            let artist_gets = if self.main.data.ipfs_nft.artist.is_some() {
                self.main.data.compute_royalty_for_artist(price)?
            } else {
                Amount::ZERO
            };
            let seller_gets = price - artist_gets;
            if seller_gets != Amount::ZERO {
                template = template.add_output(seller_gets, &self.main.data.owner, None)?;
            }
            if let Some(artist) = self
                .main
                .data
                .ipfs_nft
                .artist
                .filter(|_| artist_gets != Amount::ZERO)
            {
                template = template.add_output(artist_gets, &artist, None)?;
            }
            ret.push(Ok(template.into()));
        }
        Ok(Box::new(ret.into_iter()))
    }
}

#[cfg(test)]
mod tests;
