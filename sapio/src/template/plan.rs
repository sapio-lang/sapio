//! Ordered transaction declarations resolved before covenant commitment.
//!
//! A plan allocates a supplied budget; it does not infer the behavior of a child
//! contract by compiling it repeatedly. Funding and fee constraints are local
//! spending requirements, not additional predicates enforced by Bitcoin Script.

use super::{input::InputMetadata, OutputMeta, Template, TemplateMetadata};
use crate::contract::{Compilable, CompilationError, Context};
use crate::ordinals::{Ordinal, OrdinalPlanner};
use bitcoin::{Amount, FeeRate};
use sapio_base::policy::ScriptPolicy;
use sapio_base::timelocks::{AnyAbsTimeLock, AnyRelTimeLock};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A named minimum contribution, checked against the actual input at binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputRequirement {
    /// Name declared in the transaction plan.
    pub name: String,
    /// Minimum amount this input must supply.
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub minimum: Amount,
}

/// Local funding rules preserved after a transaction plan is frozen.
///
/// Input and output entries retain transaction order. These rules must be
/// checked when preparing a spend; they are not covenant enforcement claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FundingConstraints {
    /// Minimum contribution from each input, including the contract at zero.
    pub inputs: Vec<InputRequirement>,
    /// Declared output names, in their committed order.
    pub outputs: Vec<String>,
    /// Greatest fee accepted after the actual input values are known.
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub maximum_fee: Amount,
    /// Required final fee rate, checked against the completed transaction.
    #[serde(rename = "minimum_feerate_sat_kwu")]
    #[schemars(with = "Option<u64>")]
    pub minimum_feerate: Option<FeeRate>,
}

/// Reference to a named input in a plan.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputRef(String);

impl InputRef {
    /// The declared input name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// Reference to a named output in a plan.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputRef(String);

impl OutputRef {
    /// The declared output name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// Amount assigned to an output before its child is compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputAmount {
    /// Allocate this exact amount.
    Exact(Amount),
    /// Allocate the budget remaining after exact outputs and reserved fees.
    /// Only one output may receive the remainder.
    Remainder,
}

/// Treatment of funds beyond declared outputs and reserved fees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surplus {
    /// Require the planned budget to be allocated exactly and reject any
    /// increase in actual fees when the transaction is funded.
    #[default]
    Reject,
    /// Permit unallocated funds to become fees up to this absolute ceiling.
    Fees {
        /// Maximum total fee, including the explicit reservation.
        maximum: Amount,
    },
}

/// A declaration that cannot be resolved into a checked transaction template.
#[derive(Debug)]
pub enum PlanError {
    /// A Bitcoin transaction must declare at least one output.
    NoOutputs,
    /// Resolved funding rules are already inconsistent with the transaction.
    Funding(super::funding::FundingError),
    /// Input and output names must be nonempty.
    EmptyName,
    /// Two inputs or two outputs have the same name.
    DuplicateName {
        /// Whether the duplicate names an input or an output.
        kind: &'static str,
        /// Repeated name.
        name: String,
    },
    /// A reference does not identify an input in this plan.
    UnknownInput(String),
    /// A reference does not identify an output in this plan.
    UnknownOutput(String),
    /// Two outputs request the same residual budget.
    MultipleRemainders {
        /// Previously declared remainder output.
        first: String,
        /// Rejected additional remainder output.
        second: String,
    },
    /// A sum exceeds the representable satoshi range.
    AmountOverflow {
        /// The declaration whose amount could not be accumulated.
        at: String,
    },
    /// Exact outputs and reserved fees exceed the declared input budget.
    InsufficientFunds {
        /// Sum of the declared input contributions.
        available: Amount,
        /// Sum of all exact output amounts.
        outputs: Amount,
        /// Required fee reservation.
        fees: Amount,
    },
    /// Some planned funds have no output or explicit fee destination.
    UnallocatedFunds {
        /// Amount requiring a remainder output or an explicit surplus policy.
        amount: Amount,
    },
    /// The resolved fee is greater than the declared ceiling.
    FeeLimit {
        /// Fee required by the plan.
        fee: Amount,
        /// Greatest permitted fee.
        maximum: Amount,
    },
    /// Relative lock constraints refer to different time domains.
    ConflictingRelativeLocks {
        /// Input with incompatible requirements.
        input: String,
        /// Existing encoded sequence requirement.
        existing: u32,
        /// Rejected encoded sequence requirement.
        requested: u32,
    },
    /// Absolute lock constraints refer to different time domains.
    ConflictingAbsoluteLocks {
        /// Existing encoded lock-time requirement.
        existing: u32,
        /// Rejected encoded lock-time requirement.
        requested: u32,
    },
    /// A child would require a mixture of tracked and unknown ordinal ranges.
    UnknownOrdinalAllocation {
        /// Output requiring the unsupported allocation, or `fees`.
        at: String,
        /// Known input sats that must be allocated first.
        tracked_remaining: Amount,
    },
    /// Tracked ordinal ranges do not describe the contract input's full budget.
    OrdinalFunding {
        /// Contract input budget.
        available: Amount,
        /// Total sats described by the supplied ranges.
        tracked: Amount,
    },
    /// The supplied input ranges do not establish this ordinal's position.
    UnknownOrdinal {
        /// Output whose placement cannot be verified.
        output: String,
        /// Requested ordinal identity.
        ordinal: Ordinal,
    },
    /// An ordinal's declared offset is outside the output's allocated amount.
    InvalidOrdinalOffset {
        /// Output containing the invalid offset declaration.
        output: String,
        /// Requested zero-based satoshi offset.
        offset: u64,
        /// Output allocation.
        amount: Amount,
    },
    /// Ordered allocation does not put the ordinal at its declared position.
    OrdinalPlacement {
        /// Output whose placement requirement was violated.
        output: String,
        /// Requested ordinal identity.
        ordinal: Ordinal,
        /// Required zero-based output offset.
        required: u64,
        /// Actual offset, or `None` when the ordinal is outside this output.
        actual: Option<u64>,
    },
    /// The existing checked builder or a child contract rejected the plan.
    Compilation {
        /// Declaration being lowered.
        at: String,
        /// Underlying checked compilation error.
        source: Box<CompilationError>,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOutputs => f.write_str("transaction plan must declare at least one output"),
            Self::Funding(error) => error.fmt(f),
            Self::EmptyName => f.write_str("transaction input and output names must be nonempty"),
            Self::DuplicateName { kind, name } => write!(f, "duplicate {kind} name {name:?}"),
            Self::UnknownInput(name) => write!(f, "input {name:?} is absent from this plan"),
            Self::UnknownOutput(name) => write!(f, "output {name:?} is absent from this plan"),
            Self::MultipleRemainders { first, second } => write!(
                f,
                "outputs {first:?} and {second:?} both request the remainder; choose one"
            ),
            Self::AmountOverflow { at } => write!(f, "amount overflow while adding {at}"),
            Self::InsufficientFunds { available, outputs, fees } => write!(
                f,
                "inputs provide {} sat, but exact outputs need {} sat and fees need {} sat",
                available.to_sat(), outputs.to_sat(), fees.to_sat()
            ),
            Self::UnallocatedFunds { amount } => write!(
                f,
                "{} sat has no destination; declare a remainder output or an explicit fee surplus limit",
                amount.to_sat()
            ),
            Self::FeeLimit { fee, maximum } => write!(
                f, "resolved fee {} sat exceeds the {} sat fee limit", fee.to_sat(), maximum.to_sat()
            ),
            Self::ConflictingRelativeLocks { input, existing, requested } => write!(
                f,
                "input {input:?} mixes relative height and time requirements ({existing:#x}, {requested:#x})"
            ),
            Self::ConflictingAbsoluteLocks { existing, requested } => write!(
                f,
                "transaction mixes absolute height and time requirements ({existing}, {requested})"
            ),
            Self::UnknownOrdinalAllocation { at, tracked_remaining } => write!(
                f,
                "cannot allocate {at}: assign the remaining {} tracked sats to outputs before unknown auxiliary input sats",
                tracked_remaining.to_sat()
            ),
            Self::OrdinalFunding { available, tracked } => write!(
                f,
                "contract input provides {} sat, but its ordinal ranges describe {} sat",
                available.to_sat(), tracked.to_sat()
            ),
            Self::UnknownOrdinal { output, ordinal } => write!(
                f, "cannot verify ordinal {} in output {output:?}: its input position is unknown", ordinal.0
            ),
            Self::InvalidOrdinalOffset { output, offset, amount } => write!(
                f, "output {output:?} contains {} sat, so ordinal offset {offset} is outside it", amount.to_sat()
            ),
            Self::OrdinalPlacement { output, ordinal, required, actual } => match actual {
                Some(actual) => write!(f, "ordinal {} reaches output {output:?} at offset {actual}, but offset {required} is required", ordinal.0),
                None => write!(f, "ordinal {} is outside output {output:?}, which requires it at offset {required}", ordinal.0),
            },
            Self::Compilation { at, source } => write!(f, "cannot compile {at}: {source}"),
        }
    }
}

impl std::error::Error for PlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Funding(error) => Some(error),
            Self::Compilation { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

struct PlannedInput {
    requirement: InputRequirement,
    sequence: Option<AnyRelTimeLock>,
    metadata: InputMetadata,
}

struct PlannedOutput<'a> {
    name: String,
    amount: OutputAmount,
    contract: &'a dyn Compilable,
    metadata: OutputMeta,
    ordinals: Vec<(Ordinal, u64)>,
}

/// A finite, ordered transaction plan with explicit funding and fee intent.
///
/// Exact amounts and a single remainder are resolved before compiling children.
/// Neither output ordering nor child amounts are inferred by searching arbitrary
/// contract code. A child that cannot accept its allocation produces a named
/// error. The supplied context funds are the contract input's minimum.
pub struct TemplatePlan<'a> {
    ctx: Context,
    inputs: Vec<PlannedInput>,
    outputs: Vec<PlannedOutput<'a>>,
    reserved_fees: Amount,
    surplus: Surplus,
    lock_time: Option<AnyAbsTimeLock>,
    minimum_feerate: Option<FeeRate>,
    guards: Vec<ScriptPolicy>,
    metadata: TemplateMetadata,
}

impl<'a> TemplatePlan<'a> {
    /// Start a plan whose input zero is named `contract`.
    pub fn new(ctx: Context) -> Self {
        let input = PlannedInput {
            requirement: InputRequirement {
                name: "contract".into(),
                minimum: ctx.funds(),
            },
            sequence: None,
            metadata: InputMetadata::default(),
        };
        Self {
            ctx,
            inputs: vec![input],
            outputs: Vec::new(),
            reserved_fees: Amount::ZERO,
            surplus: Surplus::Reject,
            lock_time: None,
            minimum_feerate: None,
            guards: Vec::new(),
            metadata: TemplateMetadata::new(),
        }
    }

    /// Reference to the contract input at transaction index zero.
    pub fn contract_input(&self) -> InputRef {
        InputRef(self.inputs[0].requirement.name.clone())
    }

    /// Append an auxiliary input with a minimum contribution.
    /// A zero minimum declares a sponsor slot without inventing its funding.
    pub fn input(
        &mut self,
        name: impl Into<String>,
        minimum: Amount,
    ) -> Result<InputRef, PlanError> {
        let name = name.into();
        self.check_name(
            &name,
            "input",
            self.inputs.iter().map(|i| i.requirement.name.as_str()),
        )?;
        self.inputs.push(PlannedInput {
            requirement: InputRequirement {
                name: name.clone(),
                minimum,
            },
            sequence: None,
            metadata: InputMetadata::default(),
        });
        Ok(InputRef(name))
    }

    /// Append a named output without compiling its child yet.
    /// A remainder stays at its declared position, including before exact outputs.
    pub fn output(
        &mut self,
        name: impl Into<String>,
        amount: OutputAmount,
        contract: &'a dyn Compilable,
    ) -> Result<OutputRef, PlanError> {
        let name = name.into();
        self.check_name(
            &name,
            "output",
            self.outputs.iter().map(|o| o.name.as_str()),
        )?;
        if amount == OutputAmount::Remainder {
            if let Some(first) = self
                .outputs
                .iter()
                .find(|o| o.amount == OutputAmount::Remainder)
            {
                return Err(PlanError::MultipleRemainders {
                    first: first.name.clone(),
                    second: name,
                });
            }
        }
        self.outputs.push(PlannedOutput {
            name: name.clone(),
            amount,
            contract,
            metadata: OutputMeta::default(),
            ordinals: Vec::new(),
        });
        Ok(OutputRef(name))
    }

    fn check_name<'n>(
        &self,
        name: &str,
        kind: &'static str,
        mut names: impl Iterator<Item = &'n str>,
    ) -> Result<(), PlanError> {
        if name.is_empty() {
            Err(PlanError::EmptyName)
        } else if names.any(|previous| previous == name) {
            Err(PlanError::DuplicateName {
                kind,
                name: name.into(),
            })
        } else {
            Ok(())
        }
    }

    /// Require this input's sequence to meet a typed relative lock.
    /// Compatible constraints merge to their maximum; different domains fail.
    pub fn require_older(
        &mut self,
        input: &InputRef,
        required: AnyRelTimeLock,
    ) -> Result<&mut Self, PlanError> {
        let planned = self
            .inputs
            .iter_mut()
            .find(|i| i.requirement.name == input.0)
            .ok_or_else(|| PlanError::UnknownInput(input.0.clone()))?;
        if let Some(existing) = planned.sequence {
            if !matches!(
                (existing, required),
                (AnyRelTimeLock::RH(_), AnyRelTimeLock::RH(_))
                    | (AnyRelTimeLock::RT(_), AnyRelTimeLock::RT(_))
            ) {
                return Err(PlanError::ConflictingRelativeLocks {
                    input: input.0.clone(),
                    existing: existing.get(),
                    requested: required.get(),
                });
            }
            planned.sequence = Some(existing.max(required));
        } else {
            planned.sequence = Some(required);
        }
        Ok(self)
    }

    /// Require a typed absolute transaction lock, merging compatible minima.
    pub fn require_after(&mut self, required: AnyAbsTimeLock) -> Result<&mut Self, PlanError> {
        if let Some(existing) = self.lock_time {
            if !matches!(
                (existing, required),
                (AnyAbsTimeLock::AH(_), AnyAbsTimeLock::AH(_))
                    | (AnyAbsTimeLock::AT(_), AnyAbsTimeLock::AT(_))
            ) {
                return Err(PlanError::ConflictingAbsoluteLocks {
                    existing: existing.get(),
                    requested: required.get(),
                });
            }
            self.lock_time = Some(existing.max(required));
        } else {
            self.lock_time = Some(required);
        }
        Ok(self)
    }

    /// Reserve at least this many satoshis for fees; repeated requests take the maximum.
    pub fn reserve_fees(&mut self, minimum: Amount) -> &mut Self {
        self.reserved_fees = self.reserved_fees.max(minimum);
        self
    }

    /// Choose explicitly whether surplus funding may become additional fees.
    pub fn surplus(&mut self, surplus: Surplus) -> &mut Self {
        self.surplus = surplus;
        self
    }

    /// Require a fee rate when the final transaction's weight is known.
    /// This is retained spending policy, independent of the fee reservation.
    pub fn require_feerate(&mut self, minimum: FeeRate) -> &mut Self {
        self.minimum_feerate = Some(self.minimum_feerate.map_or(minimum, |old| old.max(minimum)));
        self
    }

    /// Add an authorization predicate to this committed template.
    /// Like `Builder::add_guard`, this is not valid on a continuation suggestion.
    pub fn guard(&mut self, guard: impl Into<ScriptPolicy>) -> &mut Self {
        self.guards.push(guard.into());
        self
    }

    /// Set optional transaction annotations without changing spending rules.
    pub fn metadata(&mut self, metadata: TemplateMetadata) -> &mut Self {
        self.metadata = metadata;
        self
    }

    /// Attach optional annotations to a named input.
    pub fn input_metadata(
        &mut self,
        input: &InputRef,
        metadata: InputMetadata,
    ) -> Result<&mut Self, PlanError> {
        let planned = self
            .inputs
            .iter_mut()
            .find(|i| i.requirement.name == input.0)
            .ok_or_else(|| PlanError::UnknownInput(input.0.clone()))?;
        planned.metadata = metadata;
        Ok(self)
    }

    /// Attach optional annotations to a named output.
    pub fn output_metadata(
        &mut self,
        output: &OutputRef,
        metadata: OutputMeta,
    ) -> Result<&mut Self, PlanError> {
        let planned = self
            .outputs
            .iter_mut()
            .find(|o| o.name == output.0)
            .ok_or_else(|| PlanError::UnknownOutput(output.0.clone()))?;
        planned.metadata = metadata;
        Ok(self)
    }

    /// Require an ordinal at a zero-based satoshi offset in this output.
    /// Placement is checked against the supplied input ranges after resolving
    /// amounts, without reordering outputs or inventing unknown ordinal data.
    pub fn require_ordinal(
        &mut self,
        output: &OutputRef,
        ordinal: Ordinal,
        offset: u64,
    ) -> Result<&mut Self, PlanError> {
        let planned = self
            .outputs
            .iter_mut()
            .find(|o| o.name == output.0)
            .ok_or_else(|| PlanError::UnknownOutput(output.0.clone()))?;
        planned.ordinals.push((ordinal, offset));
        Ok(self)
    }

    /// Resolve the finite budget and locks, compile each child once, and freeze
    /// the exact ordered transaction and its local spending requirements.
    pub fn finish(self) -> Result<Template, PlanError> {
        if self.outputs.is_empty() {
            return Err(PlanError::NoOutputs);
        }
        if let Some(ordinals) = self.ctx.get_ordinals() {
            let tracked = ordinals
                .total()
                .map_err(|source| compilation("contract input ordinals", source))?;
            if tracked != self.ctx.funds() {
                return Err(PlanError::OrdinalFunding {
                    available: self.ctx.funds(),
                    tracked,
                });
            }
        }
        let budget = self.inputs.iter().try_fold(Amount::ZERO, |sum, input| {
            sum.checked_add(input.requirement.minimum)
                .ok_or_else(|| PlanError::AmountOverflow {
                    at: format!("input {:?}", input.requirement.name),
                })
        })?;
        let exact =
            self.outputs
                .iter()
                .try_fold(Amount::ZERO, |sum, output| match output.amount {
                    OutputAmount::Remainder => Ok(sum),
                    OutputAmount::Exact(amount) => {
                        sum.checked_add(amount)
                            .ok_or_else(|| PlanError::AmountOverflow {
                                at: format!("output {:?}", output.name),
                            })
                    }
                })?;
        let remainder = budget
            .checked_sub(exact)
            .and_then(|amount| amount.checked_sub(self.reserved_fees))
            .ok_or(PlanError::InsufficientFunds {
                available: budget,
                outputs: exact,
                fees: self.reserved_fees,
            })?;
        let has_remainder = self
            .outputs
            .iter()
            .any(|output| output.amount == OutputAmount::Remainder);
        let (fees, maximum_fee) = match self.surplus {
            Surplus::Reject => {
                if !has_remainder && remainder != Amount::ZERO {
                    return Err(PlanError::UnallocatedFunds { amount: remainder });
                }
                (self.reserved_fees, self.reserved_fees)
            }
            Surplus::Fees { maximum } => {
                let fees = if has_remainder {
                    self.reserved_fees
                } else {
                    budget.checked_sub(exact).expect("checked output budget")
                };
                if fees > maximum {
                    return Err(PlanError::FeeLimit { fee: fees, maximum });
                }
                (fees, maximum)
            }
        };
        let external = budget
            .checked_sub(self.inputs[0].requirement.minimum)
            .expect("summed input budget");
        let mut output_start = 0u64;
        for output in &self.outputs {
            let amount = match output.amount {
                OutputAmount::Exact(amount) => amount,
                OutputAmount::Remainder => remainder,
            };
            for &(ordinal, offset) in &output.ordinals {
                if offset >= amount.to_sat() {
                    return Err(PlanError::InvalidOrdinalOffset {
                        output: output.name.clone(),
                        offset,
                        amount,
                    });
                }
                let mut input_start = 0u64;
                let position = self
                    .ctx
                    .get_ordinals()
                    .as_ref()
                    .and_then(|ranges| {
                        ranges.0.iter().find_map(|&(start, end)| {
                            let position = (start <= ordinal && ordinal < end)
                                .then(|| input_start + (ordinal.0 - start.0));
                            input_start += end.0 - start.0;
                            position
                        })
                    })
                    .ok_or_else(|| PlanError::UnknownOrdinal {
                        output: output.name.clone(),
                        ordinal,
                    })?;
                let actual = position
                    .checked_sub(output_start)
                    .filter(|offset| *offset < amount.to_sat());
                if actual != Some(offset) {
                    return Err(PlanError::OrdinalPlacement {
                        output: output.name.clone(),
                        ordinal,
                        required: offset,
                        actual,
                    });
                }
            }
            output_start += amount.to_sat();
        }
        let mut builder = self.ctx.template();
        for (index, input) in self.inputs.iter().enumerate() {
            if index != 0 {
                builder = builder.add_sequence();
            }
            if let Some(sequence) = input.sequence {
                builder = builder
                    .set_sequence(index as isize, sequence)
                    .map_err(|source| {
                        compilation(format!("input {:?}", input.requirement.name), source)
                    })?;
            }
        }
        if let Some(lock_time) = self.lock_time {
            builder = builder
                .set_lock_time(lock_time)
                .map_err(|source| compilation("transaction lock", source))?;
        }
        for guard in self.guards {
            builder = builder.add_guard(guard);
        }
        let mut external_pending = external != Amount::ZERO;
        if external_pending && builder.ctx().get_ordinals().is_none() {
            builder = builder
                .add_amount(external)
                .map_err(|source| compilation("auxiliary funding", source))?;
            external_pending = false;
        }
        for output in &self.outputs {
            if external_pending && builder.ctx().funds() == Amount::ZERO {
                builder = builder
                    .add_amount(external)
                    .map_err(|source| compilation("auxiliary funding", source))?;
                external_pending = false;
            }
            let amount = match output.amount {
                OutputAmount::Exact(amount) => amount,
                OutputAmount::Remainder => remainder,
            };
            if external_pending && amount > builder.ctx().funds() {
                return Err(PlanError::UnknownOrdinalAllocation {
                    at: format!("output {:?}", output.name),
                    tracked_remaining: builder.ctx().funds(),
                });
            }
            builder = builder
                .add_output(amount, output.contract, Some(output.metadata.clone()))
                .map_err(|source| compilation(format!("output {:?}", output.name), source))?;
        }
        if external_pending {
            if builder.ctx().funds() != Amount::ZERO {
                return Err(PlanError::UnknownOrdinalAllocation {
                    at: "fees".into(),
                    tracked_remaining: builder.ctx().funds(),
                });
            }
            builder = builder
                .add_amount(external)
                .map_err(|source| compilation("auxiliary funding", source))?;
        }
        let mut template: Template = builder
            .add_fees(fees)
            .map_err(|source| compilation("fees", source))?
            .into();
        template.inputs = self
            .inputs
            .iter()
            .map(|input| input.metadata.clone())
            .collect();
        template.metadata_map_s2s = self.metadata;
        template.funding_constraints = Some(FundingConstraints {
            inputs: self
                .inputs
                .into_iter()
                .map(|input| input.requirement)
                .collect(),
            outputs: self.outputs.into_iter().map(|output| output.name).collect(),
            maximum_fee,
            minimum_feerate: self.minimum_feerate,
        });
        template
            .validate_funding_constraints()
            .map_err(PlanError::Funding)?;
        Ok(template)
    }
}

fn compilation(at: impl Into<String>, source: CompilationError) -> PlanError {
    PlanError::Compilation {
        at: at.into(),
        source: Box::new(source),
    }
}
