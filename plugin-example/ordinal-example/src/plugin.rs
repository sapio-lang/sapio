use std::collections::BTreeMap;

use bitcoin::Amount;
use bitcoin::XOnlyPublicKey;
use sapio::contract::Compilable;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
use sapio::contract::Contract;
use sapio::ordinals::OrdinalPlanner;
use sapio::ordinals::OrdinalSpec;
use sapio::template::{OutputAmount, Template};
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
/// # SimpleOrdinal
/// A really Ordinal  Bearing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct SimpleOrdinal {
    ordinal: u64,
    owner: XOnlyPublicKey,
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Sell {
    purchaser: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    amount: AmountF64,
    change: AmountF64,
    fee: AmountF64,
}

impl Sell {
    fn payin(&self) -> Result<Amount, CompilationError> {
        Amount::from(self.amount)
            .checked_add(self.change.into())
            .and_then(|sum| sum.checked_add(self.fee.into()))
            .ok_or(CompilationError::OutOfFunds)
    }
}

fn multimap<T: Ord + PartialOrd + Eq + Clone, U: Clone, const N: usize>(
    v: [(T, U); N],
) -> BTreeMap<T, Vec<U>> {
    let mut ret = BTreeMap::new();
    for (t, u) in v.into_iter() {
        ret.entry(t.clone()).or_insert(vec![]).push(u)
    }
    ret
}
// ASSUMES 500 sats after Ord are "dust"
impl SimpleOrdinal {
    fn ordinal_offset(&self, ctx: &Context) -> Result<u64, CompilationError> {
        let ords = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        let mut total = 0u64;
        let mut offset = None;
        for (start, end) in &ords.0 {
            let length = end
                .0
                .checked_sub(start.0)
                .filter(|length| *length > 0)
                .ok_or_else(|| {
                    CompilationError::OrdinalsError(
                        "Ordinal ranges must be nonempty and forward".into(),
                    )
                })?;
            if (start.0..end.0).contains(&self.ordinal) {
                offset = total.checked_add(self.ordinal - start.0);
            }
            total = total
                .checked_add(length)
                .ok_or(CompilationError::OutOfFunds)?;
        }
        let mut sorted = ords.0.clone();
        sorted.sort_unstable();
        if sorted.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(CompilationError::OrdinalsError(
                "Ordinal ranges must not overlap".into(),
            ));
        }
        if total != ctx.funds().to_sat() {
            return Err(CompilationError::OrdinalsError(
                "Ordinal ranges must cover available funds exactly".into(),
            ));
        }
        let offset = offset
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Intended Ordinal".into()))?;
        if offset
            .checked_add(501)
            .filter(|required| *required <= total)
            .is_none()
        {
            return Err(CompilationError::OutOfFunds);
        }
        Ok(offset)
    }

    #[continuation(guarded_by = "[Self::signed]", web_api)]
    fn sell_with_planner(self, ctx: Context, sale: Sell) {
        let network = ctx.network;
        self.ordinal_offset(&ctx)?;
        let ords = ctx
            .get_ordinals()
            .clone()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        let plan = ords.output_plan(&OrdinalSpec {
            payouts: [Amount::from(sale.amount), sale.change.into()]
                .into_iter()
                .filter(|amount| *amount != Amount::ZERO)
                .collect(),
            payins: [sale.payin()?]
                .into_iter()
                .filter(|amount| *amount != Amount::ZERO)
                .collect(),
            fees: sale.fee.into(),
            ordinals: [Ordinal(self.ordinal)].into(),
        })?;
        let buyer: &dyn Compilable = &Compiled::from_address(
            sale.purchaser.require_network(network)?,
            bitcoin::Amount::ZERO,
        );
        plan.build_plan(
            ctx,
            multimap([
                (sale.amount.into(), (&self.owner as &dyn Compilable, None)),
                (sale.change.into(), (buyer, None)),
            ])
            .into_iter()
            .filter(|(amount, _)| *amount != Amount::ZERO)
            .collect(),
            [(Ordinal(self.ordinal), (buyer, None))].into(),
            (&self.owner, None),
        )?
        .into()
    }
    #[continuation(guarded_by = "[Self::signed]", web_api)]
    fn sell(self, ctx: Context, sale: Sell) -> Result<Template, CompilationError> {
        let network = ctx.network;
        let index = self.ordinal_offset(&ctx)?;
        let payin = sale.payin()?;
        let remaining = ctx
            .funds()
            .checked_sub(Amount::from_sat(index))
            .and_then(|amount| amount.checked_sub(Amount::from_sat(501)))
            .ok_or(CompilationError::OutOfFunds)?;
        let buyer = Compiled::from_address(
            sale.purchaser.require_network(network)?,
            bitcoin::Amount::ZERO,
        );
        let mut plan = ctx.template_plan();
        if payin != Amount::ZERO {
            plan.input("buyer_funding", payin)?;
        }
        if index != 0 {
            plan.output(
                "owner_prefix",
                OutputAmount::Exact(Amount::from_sat(index)),
                &self.owner,
            )?;
        }
        let ordinal = plan.output(
            "ordinal",
            OutputAmount::Exact(Amount::from_sat(501)),
            &buyer,
        )?;
        plan.require_ordinal(&ordinal, Ordinal(self.ordinal), 0)?;
        // Allocate the complete known ordinal input before the buyer-funded
        // price and change; those external sats have no tracked ordinal ranges.
        if remaining != Amount::ZERO {
            plan.output(
                "owner_remainder",
                OutputAmount::Exact(remaining),
                &self.owner,
            )?;
        }
        if Amount::from(sale.amount) != Amount::ZERO {
            plan.output(
                "price",
                OutputAmount::Exact(sale.amount.into()),
                &self.owner,
            )?;
        }
        if Amount::from(sale.change) != Amount::ZERO {
            plan.output(
                "buyer_change",
                OutputAmount::Exact(sale.change.into()),
                &buyer,
            )?;
        }
        plan.reserve_fees(sale.fee.into());
        Ok(plan.finish()?)
    }
}

/// # The SimpleNFT Contract
impl Contract for SimpleOrdinal {
    // Ordinals... only good for selling?
    declare! {actions, Self::sell, Self::sell_with_planner}

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        self.ordinal_offset(&ctx)?;
        Ok(ctx.funds())
    }
}

impl SimpleOrdinal {
    /// # signed
    /// Get the current owners signature.
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(self.owner.clone())
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![SimpleOrdinal, "logo.png"];

#[cfg(test)]
mod tests;
