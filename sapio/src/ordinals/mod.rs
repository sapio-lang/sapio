// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Functionality for Ordinals

use crate::{
    contract::{Compilable, CompilationError, TxTmplIt},
    template::{
        builder::{AddingFees, BuilderState},
        Builder, OutputMeta,
    },
    util::amountrange::{AmountF64, AmountU64},
    Context,
};
use bitcoin::Amount;
pub use sapio_base::plugin_args::Ordinal;
pub use sapio_base::plugin_args::OrdinalsInfo;
use serde_derive::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Struct for a payout plan
pub struct OrdinalSpec {
    /// List of payout amounts
    pub payouts: Vec<Amount>,
    /// Incoming Payments
    // TODO: Optional Ordinal info?
    pub payins: Vec<Amount>,
    /// Fees available
    pub fees: Amount,
    /// List of Ordinals
    pub ordinals: BTreeSet<Ordinal>,
}

impl OrdinalSpec {
    fn total(&self) -> Amount {
        self.payin_sum()
            + self.fees
            + self.payouts.iter().fold(Amount::ZERO, |a, b| a + *b)
            + self
                .ordinals
                .iter()
                .map(|m| m.padding() + Amount::ONE_SAT)
                .sum()
    }

    fn payin_sum(&self) -> Amount {
        self.payins.iter().fold(Amount::ZERO, |a, b| a + *b)
    }
}

/// Plan Step represents one step in a Builder Plan
#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub enum PlanStep {
    /// Must be Last, the amount of fees
    Fee(Amount),
    /// Change to send to a change addr
    Change(Amount),
    /// Payout to some value carrying contract
    Payout(Amount),
    /// Payout to an ordinal contract
    Ordinal(Ordinal),
    /// PayIn via a separate input
    PayIn(Amount),
}
/// The steps to follow to construct a transaction
pub struct Plan(Vec<PlanStep>);

impl Plan {
    /// Turn a plan into a txn...
    pub fn build_plan(
        self,
        ctx: Context,
        mut bs: BTreeMap<Amount, Vec<(&dyn Compilable, Option<OutputMeta>)>>,
        mut os: BTreeMap<Ordinal, (&dyn Compilable, Option<OutputMeta>)>,
        (change, change_meta): (&dyn Compilable, Option<OutputMeta>),
    ) -> Result<BuilderState<AddingFees>, CompilationError> {
        let mut tmpl = ctx.template();
        for step in self.0 {
            match step {
                PlanStep::Fee(f) => {
                    return tmpl.add_fees(f.into());
                }
                PlanStep::Change(amt) => {
                    tmpl = tmpl.add_output(amt.into(), change, change_meta.clone())?;
                }
                PlanStep::Payout(amt) => {
                    let vs = bs
                        .get_mut(&amt.into())
                        .ok_or(CompilationError::OrdinalsError(
                            "No Place for payout".into(),
                        ))?;
                    let (contract, metadata) = vs.pop().ok_or(CompilationError::OrdinalsError(
                        "No Place for payout".into(),
                    ))?;
                    tmpl = tmpl.add_output(amt.into(), contract, metadata)?;
                }
                PlanStep::Ordinal(o) => {
                    tmpl = {
                        let (contract, metadata) = os
                            .remove(&o)
                            .ok_or(CompilationError::OrdinalsError("No Place for Ord".into()))?;
                        tmpl.add_output(
                            o.padding() + Amount::from_sat(1),
                            contract,
                            metadata.clone(),
                        )?
                    }
                }
                PlanStep::PayIn(a) => {
                    tmpl = tmpl.add_sequence().add_amount(a);
                }
            }
        }
        Err(CompilationError::OrdinalsError("Poorly Formed Plan".into()))
    }
}

/// Generates an Output Plan
pub trait OrdinalPlanner {
    /// Computes the total sats in an OrdinalsInfo
    fn total(&self) -> Amount;
    /// Generates an Output Plan which details how Change/Payouts/Fees/Ordinals
    fn output_plan(&self, spec: &OrdinalSpec) -> Result<Plan, CompilationError>;
}
impl OrdinalPlanner for OrdinalsInfo {
    /// Computes the total sats in an OrdinalsInfo
    fn total(&self) -> Amount {
        Amount::from_sat(self.0.iter().map(|(a, b)| b.0 - a.0).sum())
    }
    /// Generates an Output Plan which details how Change/Payouts/Fees/Ordinals
    fn output_plan(&self, spec: &OrdinalSpec) -> Result<Plan, CompilationError> {
        if self.total() < spec.total() {
            return Err(CompilationError::OutOfFunds);
        } else {
            let mut payouts: VecDeque<_> = spec.payouts.iter().cloned().collect();
            {
                let v = payouts.make_contiguous();
                v.sort();
            }
            let info_master = {
                let mut info = self
                    .0
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(a, b)| (b, a))
                    .collect::<Vec<_>>();
                info[..].sort();
                info
            };
            let order = {
                let mut order = vec![];
                let mut info = info_master.clone();
                let mut it = info.iter_mut().peekable();
                let mut info_idx = 0;
                'ordscan: for ord in spec.ordinals.iter() {
                    while info_idx < info.len() {
                        if *ord >= info[info_idx].0 .0 && *ord < info[info_idx].0 .1 {
                            order.push((info[info_idx].1, (*ord)));
                            // Add the padding here to prevent bugs

                            let mut unfilled = ord.padding().as_sat() + 1;
                            while unfilled != 0 {
                                let ((lower, upper), _) = &mut info[info_idx];
                                let available = upper.0 - lower.0;
                                lower.0 += std::cmp::min(available, unfilled);
                                unfilled = unfilled.saturating_sub(available);
                                if unfilled > 0 {
                                    info_idx += 1; // advance the global index whenever exhausted
                                }
                            }

                            continue 'ordscan;
                        }
                        info_idx += 1
                    }
                    return Err(CompilationError::OrdinalsError("Ordinal Not Found".into()));
                }
                order[..].sort();
                order
            };
            let mut info = info_master.clone();
            let mut info_idx = 0;
            let mut base_instructions = vec![];
            for payin in &spec.payins {
                base_instructions.push(PlanStep::PayIn(*payin));
            }
            for (idx, ord) in order {
                let mut total = 0;

                // Fast Forward to the required IDX and count how many sats
                while info_idx < idx {
                    let ((lower, upper), _) = &mut info[info_idx];
                    total += upper.0 - lower.0;
                    info_idx += 1;
                }

                // if ord != lower, pull out those sats
                let ((lower, upper), _) = &mut info[info_idx];
                total += ord.0 - lower.0;

                // Add a Payout covering the total... this is NP Hard?
                // Greedily pick the largest and loop
                // TODO: Solver?
                greedy_assign_outs(total, &mut payouts, &mut base_instructions);

                // Advance lower bound past the end

                let mut unfilled = ord.padding().as_sat() + 1;
                while unfilled != 0 {
                    let ((lower, upper), _) = &mut info[info_idx];
                    let available = upper.0 - lower.0;
                    lower.0 += std::cmp::min(available, unfilled);
                    unfilled = unfilled.saturating_sub(available);
                    if unfilled > 0 {
                        info_idx += 1; // advance the global index whenever exhausted
                    }
                }

                base_instructions.push(PlanStep::Ordinal(ord));
            }
            let mut total = 0;

            // Fast Forward to the required IDX and count how many sats remain
            while info_idx < info.len() {
                let ((lower, upper), _) = &mut info[info_idx];
                total += upper.0 - lower.0;
                info_idx += 1;
            }

            total += spec.payin_sum().as_sat();

            // Add a Payout covering the total
            if total < spec.fees.as_sat() {
                return Err(CompilationError::OrdinalsError(
                    "Failed to save enough for Fees".into(),
                ));
            }

            greedy_assign_outs(total, &mut payouts, &mut base_instructions);
            if payouts.len() > 0 {
                return Err(CompilationError::OrdinalsError(
                    "Failed to assign all payouts".into(),
                ));
            }

            // Reserve fees now!
            total -= spec.fees.as_sat();
            // Whatever is left can become Change...
            base_instructions.push(PlanStep::Change(Amount::from_sat(total)));

            base_instructions.push(PlanStep::Fee(spec.fees.into()));

            Ok(Plan(base_instructions))
        }
    }
}

fn greedy_assign_outs(
    mut total: u64,
    payouts: &mut VecDeque<Amount>,
    base_instructions: &mut Vec<PlanStep>,
) {
    while total > 0 {
        match payouts.binary_search(&Amount::from_sat(total)) {
            Ok(i) => {
                base_instructions.push(PlanStep::Payout(Amount::from_sat(total)));
                payouts.remove(i);
                total = 0;
            }
            Err(i) => {
                if i == 0 {
                    // Best we can do is make this change, total smaller than any payout
                    // Must be change and not fee because we still have ordinals
                    base_instructions.push(PlanStep::Change(Amount::from_sat(total)));
                    total = 0;
                }

                let v = payouts.remove(i - 1).expect("Valid given not 0");
                base_instructions.push(PlanStep::Payout(v.into()));
                total -= v.as_sat();
            }
        }
    }
}
