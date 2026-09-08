// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Deterministic ordinal output planning in transaction input order.
use crate::contract::{Compilable, CompilationError};
use crate::template::builder::{AddingFees, BuilderState};
use crate::template::OutputMeta;
use crate::Context;
use bitcoin::Amount;
pub use sapio_base::plugin_args::{Ordinal, OrdinalsInfo};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Requested payouts and the separately funded auxiliary inputs.
pub struct OrdinalSpec {
    /// Positive ordinary payout amounts, each assigned to one output.
    pub payouts: Vec<Amount>,
    /// Positive auxiliary input amounts with unknown ordinal ranges.
    pub payins: Vec<Amount>,
    /// Reserved transaction fees.
    pub fees: Amount,
    /// Each requested ordinal starts its own padded output.
    pub ordinals: BTreeSet<Ordinal>,
}

fn invalid(message: &str) -> CompilationError {
    CompilationError::OrdinalsError(message.into())
}
fn sum_amounts(amounts: &[Amount]) -> Result<u64, CompilationError> {
    amounts.iter().try_fold(0u64, |sum, amount| {
        if *amount == Amount::ZERO {
            return Err(invalid("Payouts and auxiliary inputs must be positive"));
        }
        sum.checked_add(amount.as_sat())
            .ok_or_else(|| invalid("Amount sum overflow"))
    })
}

/// One step in an ordinal-preserving output plan.
#[derive(Debug, Eq, PartialEq)]
pub enum PlanStep {
    /// Last step: reserve the exact fee.
    Fee(Amount),
    /// Ordinary change, including gaps before requested ordinals.
    Change(Amount),
    /// One requested ordinary payout.
    Payout(Amount),
    /// An ordinal at offset zero of its padded output.
    Ordinal(Ordinal),
    /// An auxiliary input, added after all known input sats are allocated.
    PayIn(Amount),
}

/// A checked output plan, ordered by sat position rather than ordinal number.
pub struct Plan(Vec<PlanStep>);
impl Plan {
    /// Build the plan against the actual remaining input ranges and balances.
    pub fn build_plan(
        self,
        ctx: Context,
        mut payouts: BTreeMap<Amount, Vec<(&dyn Compilable, Option<OutputMeta>)>>,
        mut ordinals: BTreeMap<Ordinal, (&dyn Compilable, Option<OutputMeta>)>,
        (change, change_meta): (&dyn Compilable, Option<OutputMeta>),
    ) -> Result<BuilderState<AddingFees>, CompilationError> {
        if let Some(ranges) = ctx.get_ordinals() {
            if ranges.total()? != ctx.funds() {
                return Err(invalid(
                    "Actual ordinal ranges do not match available funds",
                ));
            }
        }
        let mut template = ctx.template();
        for step in self.0 {
            match step {
                PlanStep::Fee(fee) => {
                    if template.ctx().funds() != fee
                        || !ordinals.is_empty()
                        || payouts.values().any(|targets| !targets.is_empty())
                    {
                        return Err(invalid(
                            "Plan does not exhaust its funds and payout destinations",
                        ));
                    }
                    return template.add_fees(fee);
                }
                PlanStep::Change(amount) => {
                    template = template.add_output(amount, change, change_meta.clone())?;
                }
                PlanStep::Payout(amount) => {
                    let (contract, metadata) = payouts
                        .get_mut(&amount)
                        .and_then(Vec::pop)
                        .ok_or_else(|| invalid("Missing payout destination"))?;
                    template = template.add_output(amount, contract, metadata)?;
                }
                PlanStep::Ordinal(ordinal) => {
                    let next = template
                        .ctx()
                        .get_ordinals()
                        .as_ref()
                        .and_then(|info| info.0.iter().find(|(start, end)| start < end))
                        .map(|(start, _)| *start);
                    if next != Some(ordinal) {
                        return Err(invalid(
                            "Planned ordinal does not match the actual input position",
                        ));
                    }
                    let (contract, metadata) = ordinals
                        .remove(&ordinal)
                        .ok_or_else(|| invalid("Missing ordinal destination"))?;
                    template = template.add_output(
                        ordinal.padding() + Amount::ONE_SAT,
                        contract,
                        metadata,
                    )?;
                }
                PlanStep::PayIn(amount) => {
                    template = template.add_sequence().add_amount(amount)?;
                }
            }
        }
        Err(invalid("Plan has no final fee step"))
    }
}

/// Plan known ordinal ranges without changing their transaction input order.
pub trait OrdinalPlanner {
    /// Sum valid, non-overlapping half-open ranges with checked arithmetic.
    fn total(&self) -> Result<Amount, CompilationError>;
    /// Greedily fit payouts into gaps between padded ordinal outputs.
    ///
    /// This is not an optimal packing solver. Unknown auxiliary inputs come
    /// after all known sats, and must cover fees whenever they are present.
    fn output_plan(&self, spec: &OrdinalSpec) -> Result<Plan, CompilationError>;
}
impl OrdinalPlanner for OrdinalsInfo {
    fn total(&self) -> Result<Amount, CompilationError> {
        let mut sorted = self.0.clone();
        sorted.sort_unstable();
        let mut total = 0u64;
        let mut previous_end = None;
        for (start, end) in sorted {
            if start >= end || previous_end.is_some_and(|previous| start < previous) {
                return Err(invalid(
                    "Ordinal ranges must be nonempty and non-overlapping",
                ));
            }
            total = total
                .checked_add(end.0 - start.0)
                .ok_or_else(|| invalid("Ordinal total overflow"))?;
            previous_end = Some(end);
        }
        Ok(Amount::from_sat(total))
    }

    fn output_plan(&self, spec: &OrdinalSpec) -> Result<Plan, CompilationError> {
        let known = self.total()?.as_sat();
        let payins = sum_amounts(&spec.payins)?;
        let available = known
            .checked_add(payins)
            .ok_or_else(|| invalid("Input total overflow"))?;
        let mut required = sum_amounts(&spec.payouts)?
            .checked_add(spec.fees.as_sat())
            .ok_or_else(|| invalid("Payout and fee total overflow"))?;
        let mut positions = Vec::with_capacity(spec.ordinals.len());
        for ordinal in &spec.ordinals {
            let mut offset = 0u64;
            let mut position = None;
            for (start, end) in &self.0 {
                if start <= ordinal && ordinal < end {
                    position = Some(offset + (ordinal.0 - start.0));
                    break;
                }
                offset += end.0 - start.0;
            }
            let position = position.ok_or_else(|| invalid("Requested ordinal is absent"))?;
            let size = ordinal
                .padding()
                .as_sat()
                .checked_add(1)
                .ok_or_else(|| invalid("Ordinal padding overflow"))?;
            required = required
                .checked_add(size)
                .ok_or_else(|| invalid("Payout total overflow"))?;
            let end = position
                .checked_add(size)
                .ok_or_else(|| invalid("Ordinal position overflow"))?;
            if end > known {
                return Err(invalid("Known sats do not cover the ordinal's padding"));
            }
            positions.push((position, end, *ordinal));
        }
        if required > available {
            return Err(CompilationError::OutOfFunds);
        }
        positions.sort_unstable();
        let mut payouts: VecDeque<_> = spec.payouts.iter().copied().collect();
        payouts.make_contiguous().sort_unstable();
        let mut steps = Vec::new();
        let mut cursor = 0;
        for (position, end, ordinal) in positions {
            if position < cursor {
                return Err(invalid("Requested ordinal outputs overlap"));
            }
            assign_gap(position - cursor, &mut payouts, &mut steps);
            steps.push(PlanStep::Ordinal(ordinal));
            cursor = end;
        }
        let mut tail = known - cursor;
        if payins > 0 {
            assign_gap(tail, &mut payouts, &mut steps);
            steps.extend(spec.payins.iter().copied().map(PlanStep::PayIn));
            tail = payins;
        }
        let after_fee = tail
            .checked_sub(spec.fees.as_sat())
            .ok_or(CompilationError::OutOfFunds)?;
        assign_gap(after_fee, &mut payouts, &mut steps);
        if !payouts.is_empty() {
            return Err(invalid(
                "Requested payouts do not fit between ordinal outputs",
            ));
        }
        steps.push(PlanStep::Fee(spec.fees));
        Ok(Plan(steps))
    }
}

fn assign_gap(mut available: u64, payouts: &mut VecDeque<Amount>, steps: &mut Vec<PlanStep>) {
    while available > 0 {
        let fits = payouts.partition_point(|amount| amount.as_sat() <= available);
        if fits == 0 {
            steps.push(PlanStep::Change(Amount::from_sat(available)));
            return;
        }
        let amount = payouts
            .remove(fits - 1)
            .expect("partition point identifies an existing payout");
        steps.push(PlanStep::Payout(amount));
        available -= amount.as_sat();
    }
}

#[cfg(test)]
mod tests;
