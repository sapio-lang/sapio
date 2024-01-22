use std::collections::BTreeMap;

use bitcoin::util::amount::Amount;
use bitcoin::XOnlyPublicKey;
use sapio::contract::empty;
use sapio::contract::Compilable;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
use sapio::contract::Contract;
use sapio::contract::StatefulArgumentsTrait;
use sapio::ordinals::OrdinalPlanner;
use sapio::ordinals::OrdinalSpec;
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
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    owner: XOnlyPublicKey,
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Sell {
    purchaser: bitcoin::Address,
    amount: AmountF64,
    change: AmountF64,
    fee: AmountF64,
}

#[derive(JsonSchema, Serialize, Deserialize, Default)]
pub struct Sale(Option<Sell>);
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
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn sell_with_planner(self, ctx: Context, opt_sale: Sale) {
        if let Sale(Some(sale)) = opt_sale {
            if let Some(ords) = ctx.get_ordinals().clone() {
                let plan = ords.output_plan(&OrdinalSpec {
                    payouts: vec![sale.amount.into()],
                    payins: vec![Amount::from(sale.amount) + sale.fee.into() + sale.change.into()],
                    fees: sale.fee.into(),
                    ordinals: [Ordinal(self.ordinal)].into(),
                })?;
                let buyer: &dyn Compilable = &Compiled::from_address(sale.purchaser, None);
                return plan
                    .build_plan(
                        ctx,
                        multimap([
                            (sale.amount.into(), (&self.owner, None)),
                            (sale.change.into(), (buyer, None)),
                        ]),
                        [(Ordinal(self.ordinal), (buyer, None))].into(),
                        (&self.owner, None),
                    )?
                    .into();
            }
        }
        empty()
    }
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn sell(self, ctx: Context, opt_sale: Sale) {
        if let Sale(Some(sale)) = opt_sale {
            let ords = ctx
                .get_ordinals()
                .as_ref()
                .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
            let mut index = 0;
            for (a, b) in ords.0.iter() {
                if (*a..*b).contains(&Ordinal(self.ordinal)) {
                    index += self.ordinal - a.0;
                    break;
                } else {
                    index += b.0 - a.0
                }
            }
            let mut t = ctx.template();
            // TODO: Check Index calculation
            if index != 0 {
                t = t.add_output(Amount::from_sat(index - 1), &self.owner, None)?;
            }
            let buyer = Compiled::from_address(sale.purchaser, None);
            t = t.add_output(Amount::from_sat(501), &buyer, None)?;
            let remaining = t.ctx().funds();
            t = t.add_amount(sale.amount.into());
            t = t.add_output(remaining + sale.amount.into(), &self.owner, None)?;
            t = t.add_sequence();
            t = t.add_amount(sale.change.into());
            t = t.add_output(sale.change.into(), &buyer, None)?;
            t = t.add_amount(sale.fee.into());
            t.add_fees(sale.fee.into())?.into()
        } else {
            empty()
        }
    }
}
impl StatefulArgumentsTrait for Sale {}

/// # The SimpleNFT Contract
impl Contract for SimpleOrdinal {
    // Ordinals... only good for selling?
    declare! {updatable<Sale>, Self::sell, Self::sell_with_planner}

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        let ords = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        if ords.0.iter().any(|(a, b)| (*a..*b).contains(&Ordinal(self.ordinal))) {
            Ok(Amount::from_sat(1 + 500))
        } else {
            Err(CompilationError::OrdinalsError(
                "Missing Intended Ordinal".into(),
            ))
        }
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
fn default_coerce(
    k: <SimpleOrdinal as sapio::contract::Contract>::StatefulArguments,
) -> Result<Sale, CompilationError> {
    Ok(k)
}

REGISTER![SimpleOrdinal, "logo.png"];
