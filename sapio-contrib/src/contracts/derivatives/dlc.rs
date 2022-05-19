// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A collection of modules for creating derivative contracts with Sapio
use bitcoin;

use bitcoin::util::amount::Amount;

use bitcoin::XOnlyPublicKey;
use contract::*;
use sapio::template::Template;
use sapio::*;
use sapio_base::Clause;
use std::sync::Arc;

struct Event(String);
#[derive(Clone)]
struct R(bitcoin::secp256k1::PublicKey);
#[derive(Clone)]
struct X(bitcoin::secp256k1::PublicKey);
trait DLCOracle {
    fn get_r_for_event(&self, e: &Event) -> R;
    fn get_x_for_oracle(&self) -> X;
}

struct BasicOracle {
    url: String,
    /// if None, resolve via DNS / TLS
    key_cached: Option<X>,
}

impl DLCOracle for BasicOracle {
    fn get_r_for_event(&self, _event: &Event) -> R {
        unimplemented!();
    }
    fn get_x_for_oracle(&self) -> X {
        if let Some(ref x) = self.key_cached {
            x.clone()
        } else {
            unimplemented!();
        }
    }
}

type Curve = Box<dyn Fn(u32, usize) -> Result<Vec<f64>, CompilationError>>;
struct DLCContract {
    oracles: (usize, Vec<Box<dyn DLCOracle>>),
    curve: Curve,
    points: u32,
    parties: Vec<XOnlyPublicKey>,
    event: Event,
}

impl DLCContract {
    #[guard]
    fn cooperate(&self, _ctx: Context) {
        // TODO: Add a 2nd musig_cooperate that works with whatever gets standardized
        // Keep the non-musig path in case we can't do a multi-round protocol...
        Clause::And(self.parties.iter().cloned().map(Clause::Key).collect())
    }
    #[then]
    fn payout(&self, mut ctx: Context) {
        let funds = ctx.funds();
        if self.parties.len() < 2 {
            return Err(CompilationError::TerminateCompilation);
        }
        let parties: Vec<Compiled> = self
            .parties
            .iter()
            .enumerate()
            .map(|(i, k)| ctx.derive_num(i as u64).and_then(|c| k.compile(c)))
            .collect::<Result<Vec<_>, _>>()?;

        let mut tmpls: Vec<Result<Template, CompilationError>> = vec![];
        let mut new_ctx = ctx.derive_str(Arc::new("points".to_string()))?;

        let mut oracles: Vec<_> = self
            .oracles
            .1
            .iter()
            .map(|oracle| {
                let r = oracle.get_r_for_event(&self.event);
                let k = oracle.get_x_for_oracle();
                (r.0, k.0)
            })
            .collect();
        for i in 0..=self.points {
            // increment each key
            for v in oracles.iter_mut() {
                v.1 =
                    v.1.combine(&v.0)
                        .map_err(|_| CompilationError::TerminateCompilation)?;
            }
            let guard = Clause::Threshold(
                self.oracles.0,
                oracles
                    .iter()
                    .map(|(_, oracle_k)| Ok(Clause::Key(XOnlyPublicKey::from(oracle_k.clone()))))
                    .collect::<Result<Vec<_>, CompilationError>>()?,
            );
            let mut tmpl = new_ctx.derive_num(i)?.template().add_guard(guard);
            let payouts = (self.curve)(i, parties.len())?;
            if payouts.iter().sum::<f64>() != 1f64 || payouts.len() != parties.len() {
                return Err(CompilationError::TerminateCompilation);
            }
            for (party, payout) in parties.iter().zip(payouts.iter()) {
                tmpl = tmpl.add_output(
                    Amount::from_sat((funds.as_sat() as f64 * (*payout)).trunc() as u64),
                    party,
                    None,
                )?;
            }
            tmpls.push(Ok(tmpl.into()));
        }
        Ok(Box::new(tmpls.into_iter()))
    }
}

impl Contract for DLCContract {
    declare! {then, Self::payout }
    declare! {finish, Self::cooperate}
    declare! {non updatable}
}

struct StandardDLC {
    oracles: (usize, Vec<Box<dyn DLCOracle>>),
    points: u32,
    parties: [XOnlyPublicKey; 2],
    event: Event,
    curve: SplitFunctions,
}

type Offset = f64;
type Intercept = f64;
#[derive(Copy, Clone)]
enum SplitFunctions {
    /// A positive slope Linear Function
    /// from the interecept parameter to 1.0
    LinearPositive(Intercept),
    /// A Geometric starting at the intercept parameter to 1.0
    GeometricPositive(Intercept),
    /// offset is subtracted from points to allow moving the center
    /// let g = x when sigmoid(x) == 0.5
    /// a positive offset means that h>g, where h = x when sigmoid(x-offset) == 0.5
    /// a negative offset means that h<g, where h = x when sigmoid(x-offset) == 0.5
    Sigmoid(Offset),
    // TODO:
    // Custom(MiniCalcLanguage)
}

impl SplitFunctions {
    fn get_curve(self, points: u32) -> Curve {
        match self {
            SplitFunctions::LinearPositive(b) => {
                let m = (1.0 - b) / (points as f64);
                Box::new(move |point: u32, n: usize| {
                    if n != 2 {
                        return Err(CompilationError::TerminateCompilation);
                    }
                    let r = m * (point as f64) + b;
                    Ok(vec![r, 1.0 - r])
                })
            }
            SplitFunctions::GeometricPositive(p) => {
                // p*j**points = 1
                // j**points = 1.0/p
                // log(j**points) = log(1.0/p)
                // points *log(j) = log(1.0/p)
                // log(j) = log(1.0/p)/points
                // j = 2**log(1.0/p)/points
                let j = ((1.0 / p).log2() / (points as f64)).exp2();
                Box::new(move |point: u32, n: usize| {
                    if n != 2 {
                        return Err(CompilationError::TerminateCompilation);
                    }
                    let r = p * j.powi(point as i32);
                    Ok(vec![r, 1.0 - r])
                })
            }
            SplitFunctions::Sigmoid(offset) => Box::new(move |point: u32, n: usize| {
                if n != 2 {
                    return Err(CompilationError::TerminateCompilation);
                }
                let r = 1.0 + (1.0 / (-(point as f64 - offset)).exp());
                Ok(vec![r, 1.0 - r])
            }),
        }
    }
}

impl From<StandardDLC> for DLCContract {
    fn from(s: StandardDLC) -> DLCContract {
        let curve = s.curve.get_curve(s.points);
        DLCContract {
            oracles: s.oracles,
            curve,
            points: s.points,
            parties: s.parties.into(),
            event: s.event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;
    use sapio_base::effects::EffectPath;
    use sapio_ctv_emulator_trait::CTVAvailable;
    use std::convert::TryFrom;
    struct CachedOracle {
        key: X,
        event: R,
    }

    impl DLCOracle for CachedOracle {
        fn get_r_for_event(&self, event: &Event) -> R {
            self.event.clone()
        }
        fn get_x_for_oracle(&self) -> X {
            self.key.clone()
        }
    }

    #[test]
    fn create_dlc() {
        let secp = Secp256k1::new();
        let mut rng = rand::thread_rng();
        let (_, pk1) = secp.generate_keypair(&mut rng);
        let (_, pk2) = secp.generate_keypair(&mut rng);
        let o = CachedOracle {
            key: X(pk1),
            event: R(pk2),
        };
        let (_, pk_a) = secp.generate_keypair(&mut rng);
        let (_, pk_b) = secp.generate_keypair(&mut rng);
        let d: DLCContract = StandardDLC {
            oracles: (1, vec![Box::new(o)]),
            points: 1000,
            parties: [XOnlyPublicKey::from(pk_a), XOnlyPublicKey::from(pk_b)],
            event: Event("whatever".into()),
            curve: SplitFunctions::LinearPositive(0.1),
        }
        .into();
        // Inner closure, the actual test
        let ctx = Context::new(
            bitcoin::network::constants::Network::Bitcoin,
            Amount::from_sat(1000000000),
            Arc::new(CTVAvailable),
            EffectPath::try_from("dlc").unwrap(),
            Arc::new(Default::default()),
        );
        let _r = d.compile(ctx).unwrap();
    }
}
