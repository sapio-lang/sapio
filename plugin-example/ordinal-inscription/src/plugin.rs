use bitcoin::Address;
use bitcoin::Amount;
use bitcoin::XOnlyPublicKey;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
use sapio::contract::Contract;
use sapio::ordinals::Ordinal;
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::Clause;
#[cfg(target_arch = "wasm32")]
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
/// # SimpleInscription
/// A really Ordinal  Bearing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct InscribingStep {
    owner: XOnlyPublicKey,
    data: Vec<u8>,
    content_type: String,
}
impl InscribingStep {
    fn ordinal_info(ctx: &Context) -> Result<(Ordinal, Amount), CompilationError> {
        let ords = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        let first = ords
            .0
            .first()
            .ok_or_else(|| CompilationError::OrdinalsError("Empty Ordinals Info".into()))?
            .0;
        let mut total = Amount::ZERO;
        for (start, end) in &ords.0 {
            let length = end.0.checked_sub(start.0).filter(|length| *length > 0);
            let length = length.ok_or_else(|| {
                CompilationError::OrdinalsError(
                    "Ordinal ranges must be nonempty and forward".into(),
                )
            })?;
            total = total.checked_add(Amount::from_sat(length)).ok_or_else(|| {
                CompilationError::OrdinalsError("Ordinal range total overflows".into())
            })?;
        }
        let mut sorted = ords.0.clone();
        sorted.sort_unstable();
        if sorted.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(CompilationError::OrdinalsError(
                "Ordinal ranges must not overlap".into(),
            ));
        }
        if total != ctx.funds() {
            return Err(CompilationError::OrdinalsError(
                "Ordinal ranges must cover the available funds exactly".into(),
            ));
        }
        Ok((first, total))
    }

    /// # signed
    /// Get the current owners signature.
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(self.owner)
    }

    #[guard]
    fn inscription(self, _ctx: Context) {
        let insc = sapio_base::miniscript::ord::Inscription::new(
            Some(self.content_type.as_bytes().into()),
            Some(self.data.clone()),
        );
        Clause::Inscribe(Box::new(insc), Clause::Trivial.into())
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Reveal {
    fee: AmountU64,
    alternative: Option<Address<bitcoin::address::NetworkUnchecked>>,
}
impl Default for Reveal {
    fn default() -> Self {
        Self {
            fee: Amount::from_sat(500).into(),
            alternative: None,
        }
    }
}
// ASSUMES 500 sats after Ord are "dust"
impl InscribingStep {
    fn default_reveal(&self, ctx: Context) -> sapio::contract::TxTmplIt {
        self.continue_reveal(ctx, Reveal::default())
    }

    #[continuation(
        guarded_by = "[Self::signed, Self::inscription]",
        web_api,
        defaults = "Self::default_reveal"
    )]
    fn reveal(self, ctx: Context, reveal: Reveal) {
        let network = ctx.network;
        let (ord, _) = Self::ordinal_info(&ctx)?;
        let send_with = ctx
            .funds()
            .checked_sub(reveal.fee.into())
            .ok_or(CompilationError::OutOfFunds)?;
        if send_with < ord.padding() + Amount::ONE_SAT {
            return Err(CompilationError::OutOfFunds);
        }
        let tmpl = ctx.template();
        if let Some(address) = reveal.alternative {
            tmpl.add_output(
                send_with,
                &Compiled::from_address(address.require_network(network)?, bitcoin::Amount::ZERO),
                None,
            )
        } else {
            tmpl.add_output(send_with, &self.owner, None)
        }?
        .add_fees(reveal.fee.into())?
        .into()
    }
}

/// # The SimpleInscription Contract
impl Contract for InscribingStep {
    declare! {actions, Self::reveal}

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        Self::ordinal_info(&ctx).map(|(_, amount)| amount)
    }
}

#[cfg(target_arch = "wasm32")]
REGISTER![InscribingStep, "logo.png"];

#[cfg(test)]
mod tests;
