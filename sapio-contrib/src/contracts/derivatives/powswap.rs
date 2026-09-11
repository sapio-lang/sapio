// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Contract for PowSwap Hashrate Derivatives
use bitcoin::Amount;
use bitcoin::PublicKey;
use sapio::contract::*;
use sapio::template::{Builder, Template};
use sapio::util::amountrange::AmountU64;
use sapio::*;
use sapio_base::timelocks::{AnyAbsTimeLock, AnyRelTimeLock, AnyTimeLock};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

/// A single timelock kind, or relative height plus absolute time (and vice versa).
/// Repeated constraints of the same kind use the strongest constraint.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy)]
#[serde(try_from = "Vec<AnyTimeLock>", into = "Vec<AnyTimeLock>")]
#[schemars(with = "Vec<AnyTimeLock>")]
pub struct ContractVariant(Option<AnyRelTimeLock>, Option<AnyAbsTimeLock>);

impl TryFrom<Vec<AnyTimeLock>> for ContractVariant {
    type Error = &'static str;
    fn try_from(locks: Vec<AnyTimeLock>) -> Result<Self, Self::Error> {
        let mut relative = None;
        let mut absolute = None;
        for lock in locks {
            match lock {
                AnyTimeLock::R(r) => {
                    if let Some(old) = relative {
                        if !matches!(
                            (old, r),
                            (AnyRelTimeLock::RH(_), AnyRelTimeLock::RH(_))
                                | (AnyRelTimeLock::RT(_), AnyRelTimeLock::RT(_))
                        ) {
                            return Err("Mixed relative height and time");
                        }
                    }
                    relative = Some(relative.map_or(r, |old| std::cmp::max(old, r)));
                }
                AnyTimeLock::A(a) => {
                    if let Some(old) = absolute {
                        if !matches!(
                            (old, a),
                            (AnyAbsTimeLock::AH(_), AnyAbsTimeLock::AH(_))
                                | (AnyAbsTimeLock::AT(_), AnyAbsTimeLock::AT(_))
                        ) {
                            return Err("Mixed absolute height and time");
                        }
                    }
                    absolute = Some(absolute.map_or(a, |old| std::cmp::max(old, a)));
                }
            }
        }
        if matches!(
            (relative, absolute),
            (None, None)
                | (Some(AnyRelTimeLock::RH(_)), Some(AnyAbsTimeLock::AH(_)))
                | (Some(AnyRelTimeLock::RT(_)), Some(AnyAbsTimeLock::AT(_)))
        ) {
            return Err("Use one timelock kind, or relative/absolute locks with different units");
        }
        Ok(Self(relative, absolute))
    }
}
impl From<ContractVariant> for Vec<AnyTimeLock> {
    fn from(v: ContractVariant) -> Self {
        v.0.map(AnyTimeLock::R)
            .into_iter()
            .chain(v.1.map(AnyTimeLock::A))
            .collect()
    }
}

/// An output paid by a PowSwap outcome.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Pays {
    /// Payment in satoshis.
    pub sats: AmountU64,
    /// Recipient public key, paid through its x-only Taproot output.
    #[schemars(with = "String")]
    pub to: PublicKey,
}
/// One timelocked settlement of a PowSwap.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Outcome {
    /// Settlement timelocks.
    pub unlocks_if: ContractVariant,
    /// Nonempty list of positive payments.
    pub outcome: Vec<Pays>,
}
/// Two competing height/time settlements with a cooperative signing path.
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct PowSwap {
    /// Both outcomes must distribute the same collateral.
    pub outcomes: [Outcome; 2],
    /// Distinct cooperating signers; at least two are required.
    #[schemars(with = "Vec<String>")]
    pub coop: Vec<PublicKey>,
}

impl PowSwap {
    fn collateral(&self) -> Result<Amount, CompilationError> {
        if self.coop.len() < 2
            || self
                .coop
                .iter()
                .map(|k| bitcoin::XOnlyPublicKey::from(k.inner))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.coop.len()
        {
            return Err(CompilationError::TerminateWith(
                "PowSwap requires distinct cooperating signers".into(),
            ));
        }
        let mut sums = [0u64; 2];
        for (sum, outcome) in sums.iter_mut().zip(&self.outcomes) {
            if outcome.outcome.is_empty() {
                return Err(CompilationError::TerminateWith(
                    "Empty PowSwap outcome".into(),
                ));
            }
            for payment in &outcome.outcome {
                let value = Amount::from(payment.sats.clone()).to_sat();
                if value == 0 {
                    return Err(CompilationError::TerminateWith(
                        "Zero PowSwap payment".into(),
                    ));
                }
                *sum = sum.checked_add(value).ok_or_else(|| {
                    CompilationError::TerminateWith("PowSwap amount overflow".into())
                })?;
            }
        }
        if sums[0] != sums[1] {
            return Err(CompilationError::TerminateWith(
                "PowSwap outcomes require equal collateral".into(),
            ));
        }
        Ok(Amount::from_sat(sums[0]))
    }
    fn make_payoffs(&self, ctx: Context, payments: &[Pays]) -> Result<Builder, CompilationError> {
        let mut bld = ctx.template();
        for Pays { sats, to } in payments {
            bld = bld.add_output(
                sats.clone().into(),
                &bitcoin::XOnlyPublicKey::from(to.inner),
                None,
            )?;
        }
        Ok(bld)
    }
    #[then]
    fn payoff(self, mut base_ctx: Context) {
        let mut ret: Vec<Result<Template, CompilationError>> = vec![];
        for (i, path) in self.outcomes.iter().enumerate() {
            let ctx = base_ctx.derive_num(i as u64)?;
            let mut builder = self.make_payoffs(ctx, &path.outcome)?;
            if let Some(relative) = path.unlocks_if.0 {
                builder = builder.set_sequence(0, relative)?;
            }
            if let Some(absolute) = path.unlocks_if.1 {
                builder = builder.set_lock_time(absolute)?;
            }
            ret.push(Ok(builder.into()));
        }
        Ok(Box::new(ret.into_iter()))
    }
    #[guard]
    fn cooperate(self, _ctx: Context) {
        Clause::And(
            self.coop
                .iter()
                .map(|k| Clause::Key(k.inner.into()).into())
                .collect(),
        )
    }
}
impl Contract for PowSwap {
    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        self.collateral()
    }
    declare! {then, Self::payoff}
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::context;
    use sapio_base::timelocks::{AbsHeight, AbsTime, RelHeight, RelTime};
    fn key(n: u8) -> PublicKey {
        PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(
            &bitcoin::secp256k1::Secp256k1::new(),
            &bitcoin::secp256k1::SecretKey::from_slice(&[n; 32]).unwrap(),
        ))
    }
    fn swap() -> PowSwap {
        PowSwap {
            outcomes: [
                Outcome {
                    unlocks_if: vec![
                        AnyTimeLock::R(RelHeight::from(10).into()),
                        AnyTimeLock::A(AbsTime::try_from(600_000_000).unwrap().into()),
                    ]
                    .try_into()
                    .unwrap(),
                    outcome: vec![Pays {
                        sats: Amount::from_sat(2000).into(),
                        to: key(1),
                    }],
                },
                Outcome {
                    unlocks_if: vec![
                        AnyTimeLock::R(RelTime::from(3).into()),
                        AnyTimeLock::A(AbsHeight::try_from(100).unwrap().into()),
                    ]
                    .try_into()
                    .unwrap(),
                    outcome: vec![Pays {
                        sats: Amount::from_sat(2000).into(),
                        to: key(2),
                    }],
                },
            ],
            coop: vec![key(1), key(2)],
        }
    }
    #[test]
    fn settlements_preserve_amount_destination_and_locks() {
        let value = serde_json::to_value(swap()).unwrap();
        let swap: PowSwap = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&swap).unwrap(), value);
        let compiled = swap.compile(context(2000)).unwrap();
        assert_eq!(compiled.ctv_to_tx.len(), 2);
        for template in compiled.ctv_to_tx.values() {
            assert_eq!(template.tx.output.len(), 1);
            assert_eq!(template.tx.output[0].value.to_sat(), 2000);
            let (sequence, signer) = if template.tx.lock_time.to_consensus_u32() == 600_000_000 {
                (10, key(1))
            } else {
                assert_eq!(template.tx.lock_time.to_consensus_u32(), 100);
                ((1 << 22) | 3, key(2))
            };
            assert_eq!(template.tx.input[0].sequence.to_consensus_u32(), sequence);
            let recipient = bitcoin::XOnlyPublicKey::from(signer.inner)
                .compile(context(2000))
                .unwrap();
            assert_eq!(
                template.tx.output[0].script_pubkey,
                bitcoin::ScriptBuf::from(&recipient.address)
            );
        }
        assert!(swap.compile(context(1999)).is_err());
    }
    #[test]
    fn timelock_combinations_and_invalid_outcomes() {
        let rh = AnyTimeLock::R(RelHeight::from(1).into());
        let rt = AnyTimeLock::R(RelTime::from(1).into());
        let ah = AnyTimeLock::A(AbsHeight::try_from(1).unwrap().into());
        let at = AnyTimeLock::A(AbsTime::try_from(600_000_000).unwrap().into());
        for locks in [
            vec![rh],
            vec![rt],
            vec![ah],
            vec![at],
            vec![rh, at],
            vec![rt, ah],
        ] {
            assert!(ContractVariant::try_from(locks).is_ok());
        }
        for locks in [
            vec![],
            vec![rh, rt],
            vec![ah, at],
            vec![rh, ah],
            vec![rt, at],
        ] {
            assert!(ContractVariant::try_from(locks).is_err());
        }
        let max =
            ContractVariant::try_from(vec![rh, AnyTimeLock::R(RelHeight::from(9).into())]).unwrap();
        assert_eq!(max.0.unwrap().get(), 9);
        let mut v = swap();
        v.coop.clear();
        assert!(v.compile(context(2000)).is_err());
        let mut v = swap();
        v.coop[1] = v.coop[0];
        assert!(v.compile(context(2000)).is_err());
        let mut v = swap();
        v.outcomes[0].outcome.clear();
        assert!(v.compile(context(2000)).is_err());
        let mut v = swap();
        v.outcomes[0].outcome[0].sats = Amount::from_sat(1999).into();
        assert!(v.compile(context(2000)).is_err());
        let mut v = swap();
        v.outcomes[0].outcome.push(Pays {
            sats: Amount::from_sat(u64::MAX).into(),
            to: key(1),
        });
        assert!(v.compile(context(2000)).is_err());
    }
}
