use bitcoin::hashes::hex::ToHex;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::amount::Amount;
use bitcoin::XOnlyPublicKey;
use sapio::contract::empty;
use sapio::contract::object::ObjectMetadata;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
use sapio::contract::Contract;
use sapio::contract::StatefulArgumentsTrait;
use sapio::util::amountrange::AmountF64;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_nft_trait::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::sync::Arc;
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
// ASSUMES 500 sats after Ord are "dust"
impl SimpleOrdinal {
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn sell(self, ctx: Context, opt_sale: Sale) {
        if let Sale(Some(sale)) = opt_sale {
            let o = ctx
                .get_ordinals()
                .as_ref()
                .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
            let mut index = 0;
            for (a, b) in o.iter() {
                if (*a..*b).contains(&self.ordinal) {
                    index += (self.ordinal) - a;
                    break;
                } else {
                    index += b - a
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
    declare! {updatable<Sale>, Self::sell}

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        let ords = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        if ords.iter().any(|(a, b)| (*a..*b).contains(&self.ordinal)) {
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
