use bitcoin::util::amount::Amount;
use bitcoin::Address;
use bitcoin::XOnlyPublicKey;
use sapio::contract::CompilationError;
use sapio::contract::Compiled;
use sapio::contract::Contract;
use sapio::contract::StatefulArgumentsTrait;
use sapio::ordinals::OrdinalPlanner;
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
/// # SimpleInscription
/// A really Ordinal  Bearing Contract
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct InscribingStep {
    #[schemars(with = "bitcoin::hashes::sha256::Hash")]
    owner: XOnlyPublicKey,
    data: Vec<u8>,
    content_type: String,
}
impl InscribingStep {
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
        Clause::Inscribe(Box::new(insc), Box::new(Clause::Trivial))
    }
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Reveal {
    fee: AmountU64,
    alternative: Option<Address>,
}
impl Reveal {
    fn parse(
        k: <InscribingStep as sapio::contract::Contract>::StatefulArguments,
    ) -> Result<Self, CompilationError> {
        Ok(k)
    }
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
    #[continuation(
        guarded_by = "[Self::signed, Self::inscription]",
        web_api,
        coerce_args = "Reveal::parse"
    )]
    fn reveal(self, ctx: Context, reveal: Reveal) {
        let ord = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?
            .0[0]
            .0;
        let funds = ctx.funds();
        if funds < Amount::from(reveal.fee) + ord.padding() + Amount::ONE_SAT {
            return Err(CompilationError::OutOfFunds);
        }
        let send_with = funds - reveal.fee.into();
        let tmpl = ctx.template();
        if let Some(address) = reveal.alternative {
            tmpl.add_output(send_with, &Compiled::from_address(address, None), None)
        } else {
            tmpl.add_output(send_with, &self.owner, None)
        }?
        .add_fees(reveal.fee.into())?
        .into()
    }
}
impl StatefulArgumentsTrait for Reveal {}

/// # The SimpleInscription Contract
impl Contract for InscribingStep {
    declare! {updatable<Reveal>, Self::reveal}

    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        // Optional if we want to require ordinal info provided -- we can
        // happily track ordinals abstractly with some future patches.
        let ords = ctx
            .get_ordinals()
            .as_ref()
            .ok_or_else(|| CompilationError::OrdinalsError("Missing Ordinals Info".into()))?;
        Ok(ords.total())
    }
}


REGISTER![InscribingStep, "logo.png"];
