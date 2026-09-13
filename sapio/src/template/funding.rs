//! Check retained transaction-plan constraints against supplied funding.

use super::Template;
use bitcoin::{psbt::Psbt, Amount};
use sapio_base::CTVHash;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

/// Funding and fee information, including obligations that remain unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FundingReport {
    /// Sum of all available previous-output amounts.
    pub known_input_sats: u64,
    /// Exact committed output total.
    pub output_sats: u64,
    /// Minimum fee reserved by the constructor.
    pub reserved_fee_sats: u64,
    /// Input indexes for which no funding evidence was supplied.
    pub missing_inputs: Vec<usize>,
    /// Actual fee, known only when all input amounts are present.
    pub actual_fee_sats: Option<u64>,
    /// Explicit local fee cap, if declared by a transaction plan.
    pub maximum_fee_sats: Option<u64>,
    /// Minimum local fee rate in satoshis per 1,000 weight units.
    pub minimum_feerate_sat_kwu: Option<u64>,
    /// Observed weight when all final scriptSig/witness fields are supplied.
    /// This measures the transaction; it does not verify those witnesses.
    pub observed_weight_wu: Option<u64>,
    /// A requested fee rate still needs final witness sizes.
    pub fee_rate_pending: bool,
}

/// A transaction or its funding does not satisfy a retained local constraint.
#[derive(Debug)]
pub enum FundingError {
    /// Serialized constraints are inconsistent with the template.
    InvalidConstraints(String),
    /// The funded transaction changes a committed template field.
    TransactionMismatch,
    /// The supplied amounts do not correspond to every input.
    InputCount,
    /// Funding evidence fails authentication or structural checks.
    Evidence(sapio_base::psbt::FundingError),
    /// A named input cannot supply its declared contribution.
    UnderfundedInput {
        /// Transaction input index.
        index: usize,
        /// Author-assigned funding role.
        name: String,
        /// Authenticated value of the spent output.
        available: Amount,
        /// Minimum contribution declared for this input.
        required: Amount,
    },
    /// Aggregate values or a fee calculation overflow.
    Overflow,
    /// The transaction cannot pay outputs and reserved fees.
    InsufficientFunding {
        /// Authenticated aggregate input value.
        available: Amount,
        /// Value needed for outputs and reserved fees.
        required: Amount,
    },
    /// Additional funding would exceed the author's explicit fee cap.
    ExcessiveFee {
        /// Fee implied by input and output values.
        actual: Amount,
        /// Author's explicit fee cap.
        maximum: Amount,
    },
    /// The observed complete witness weight violates the requested fee rate.
    InsufficientFeeRate {
        /// Fee implied by authenticated inputs and outputs.
        fee: Amount,
        /// Minimum fee at the observed transaction weight.
        required: Amount,
        /// Complete transaction weight, including witnesses.
        weight_wu: u64,
    },
}

impl fmt::Display for FundingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstraints(reason) => write!(f, "invalid funding constraints: {reason}"),
            Self::TransactionMismatch => {
                f.write_str("transaction changes the template's committed fields")
            }
            Self::InputCount => {
                f.write_str("funding evidence must correspond to every template input")
            }
            Self::Evidence(error) => error.fmt(f),
            Self::UnderfundedInput {
                index,
                name,
                available,
                required,
            } => write!(
                f,
                "input {index} ({name}) provides {} sat but requires {} sat",
                available.to_sat(),
                required.to_sat()
            ),
            Self::Overflow => f.write_str("funding or fee arithmetic overflows"),
            Self::InsufficientFunding {
                available,
                required,
            } => write!(
                f,
                "inputs provide {} sat; outputs and reserved fees require {} sat",
                available.to_sat(),
                required.to_sat()
            ),
            Self::ExcessiveFee { actual, maximum } => write!(
                f,
                "fee is at least {} sat, exceeding the {} sat cap",
                actual.to_sat(),
                maximum.to_sat()
            ),
            Self::InsufficientFeeRate {
                fee,
                required,
                weight_wu,
            } => write!(
                f,
                "fee is {} sat; observed weight {weight_wu} wu requires {} sat",
                fee.to_sat(),
                required.to_sat()
            ),
        }
    }
}

impl std::error::Error for FundingError {}

impl Template {
    /// Check the internal consistency of retained local funding constraints.
    pub fn validate_funding_constraints(&self) -> Result<(), FundingError> {
        let Some(constraints) = &self.funding_constraints else {
            return Ok(());
        };
        let invalid = |reason: &str| FundingError::InvalidConstraints(reason.into());
        if constraints.inputs.len() != self.tx.input.len() || constraints.inputs.is_empty() {
            return Err(invalid(
                "input requirements do not match transaction inputs",
            ));
        }
        if constraints.outputs.len() != self.tx.output.len() {
            return Err(invalid("output names do not match transaction outputs"));
        }
        for names in [
            constraints
                .inputs
                .iter()
                .map(|input| input.name.as_str())
                .collect::<Vec<_>>(),
            constraints.outputs.iter().map(String::as_str).collect(),
        ] {
            let mut seen = BTreeSet::new();
            if names
                .iter()
                .any(|name| name.is_empty() || !seen.insert(*name))
            {
                return Err(invalid(
                    "input and output names must be nonempty and unique within each group",
                ));
            }
        }
        let total = constraints
            .inputs
            .iter()
            .try_fold(Amount::ZERO, |sum, input| sum.checked_add(input.minimum))
            .ok_or(FundingError::Overflow)?;
        if total != self.max || constraints.inputs[0].minimum != self.required_input_amount {
            return Err(invalid(
                "named contributions do not match the template funding budget",
            ));
        }
        let outputs = self.checked_output_total()?;
        let reserved = self
            .max
            .checked_sub(outputs)
            .ok_or_else(|| invalid("outputs exceed the funding budget"))?;
        if constraints.maximum_fee < reserved {
            return Err(invalid("fee cap is smaller than reserved fees"));
        }
        if let Some(rate) = constraints.minimum_feerate {
            // The final scriptSig and witness can only increase this base
            // weight. Reject contradictions already known before signing.
            let weight = (self.tx.base_size() as u64)
                .checked_mul(4)
                .map(bitcoin::Weight::from_wu)
                .ok_or(FundingError::Overflow)?;
            let required = rate.fee_wu(weight).ok_or(FundingError::Overflow)?;
            if required > constraints.maximum_fee {
                return Err(FundingError::InvalidConstraints(format!(
                    "fee cap {} sat is below the {} sat required by the unsigned transaction at the minimum fee rate",
                    constraints.maximum_fee.to_sat(), required.to_sat(),
                )));
            }
        }
        Ok(())
    }

    fn checked_output_total(&self) -> Result<Amount, FundingError> {
        self.tx
            .output
            .iter()
            .try_fold(Amount::ZERO, |sum, output| sum.checked_add(output.value))
            .ok_or(FundingError::Overflow)
    }

    /// Check ordered funding amounts already authenticated by the caller.
    /// Missing inputs remain explicit. This performs no signing or chain lookup.
    pub fn check_funding_amounts(
        &self,
        amounts: &[Option<Amount>],
    ) -> Result<FundingReport, FundingError> {
        self.validate_funding_constraints()?;
        if amounts.len() != self.tx.input.len() {
            return Err(FundingError::InputCount);
        }
        let outputs = self.checked_output_total()?;
        let reserved = self
            .max
            .checked_sub(outputs)
            .ok_or_else(|| FundingError::InvalidConstraints("outputs exceed the budget".into()))?;
        let mut known = Amount::ZERO;
        let mut missing = vec![];
        for (index, amount) in amounts.iter().enumerate() {
            if let Some(amount) = amount {
                if let Some(constraints) = &self.funding_constraints {
                    let input = &constraints.inputs[index];
                    if *amount < input.minimum {
                        return Err(FundingError::UnderfundedInput {
                            index,
                            name: input.name.clone(),
                            available: *amount,
                            required: input.minimum,
                        });
                    }
                }
                known = known.checked_add(*amount).ok_or(FundingError::Overflow)?;
            } else {
                missing.push(index);
            }
        }
        let maximum = self
            .funding_constraints
            .as_ref()
            .map(|constraints| constraints.maximum_fee);
        if let Some(maximum) = maximum {
            if known.checked_sub(outputs).is_some_and(|fee| fee > maximum) {
                return Err(FundingError::ExcessiveFee {
                    actual: known - outputs,
                    maximum,
                });
            }
        }
        let actual = if missing.is_empty() {
            if known < self.max {
                return Err(FundingError::InsufficientFunding {
                    available: known,
                    required: self.max,
                });
            }
            Some((known - outputs).to_sat())
        } else {
            None
        };
        let rate = self
            .funding_constraints
            .as_ref()
            .and_then(|constraints| constraints.minimum_feerate);
        Ok(FundingReport {
            known_input_sats: known.to_sat(),
            output_sats: outputs.to_sat(),
            reserved_fee_sats: reserved.to_sat(),
            missing_inputs: missing,
            actual_fee_sats: actual,
            maximum_fee_sats: maximum.map(Amount::to_sat),
            minimum_feerate_sat_kwu: rate.map(|rate| rate.to_sat_per_kwu()),
            observed_weight_wu: None,
            fee_rate_pending: rate.is_some(),
        })
    }

    /// Check a candidate's layout, funding evidence and retained local fee rules.
    ///
    /// A fee-rate requirement remains pending until all final scriptSig/witness
    /// fields are supplied. Witness validity and chain maturity are separate
    /// checks; this method never treats a construction constraint as Script.
    pub fn check_funded_psbt(&self, psbt: &Psbt) -> Result<FundingReport, FundingError> {
        let prevouts = sapio_base::psbt::previous_outputs(psbt).map_err(FundingError::Evidence)?;
        if self.ctv_index != 0
            || psbt.unsigned_tx.get_ctv_hash(self.ctv_index) != self.ctv
            || self.tx.get_ctv_hash(self.ctv_index) != self.ctv
        {
            return Err(FundingError::TransactionMismatch);
        }
        let mut report = self.check_funding_amounts(
            &prevouts
                .iter()
                .map(|output| output.map(|output| output.value))
                .collect::<Vec<_>>(),
        )?;
        if psbt
            .inputs
            .iter()
            .all(|input| input.final_script_sig.is_some() || input.final_script_witness.is_some())
        {
            let mut transaction = psbt.unsigned_tx.clone();
            for (input, data) in transaction.input.iter_mut().zip(&psbt.inputs) {
                input.script_sig = data.final_script_sig.clone().unwrap_or_default();
                input.witness = data.final_script_witness.clone().unwrap_or_default();
            }
            let weight = transaction.weight().to_wu();
            report.observed_weight_wu = Some(weight);
            if let (Some(rate), Some(fee)) =
                (report.minimum_feerate_sat_kwu, report.actual_fee_sats)
            {
                let required = rate
                    .checked_mul(weight)
                    .and_then(|value| value.checked_add(999))
                    .ok_or(FundingError::Overflow)?
                    / 1_000;
                if fee < required {
                    return Err(FundingError::InsufficientFeeRate {
                        fee: Amount::from_sat(fee),
                        required: Amount::from_sat(required),
                        weight_wu: weight,
                    });
                }
                report.fee_rate_pending = false;
            }
        }
        Ok(report)
    }
}
