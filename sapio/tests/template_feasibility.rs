#[path = "fixtures/covenant.rs"]
mod covenant;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::{Compilable, CompilationError, Contract};
use sapio::template::Template;
use sapio::{declare, guard, then, Context};
use sapio_base::covenant::{Ctv, Emulatable, LoweringPlan};
use sapio_base::policy::ScriptPolicy;
use sapio_base::{CTVHash, Clause};
use std::sync::Arc;

fn key(index: u8) -> XOnlyPublicKey {
    SecretKey::from_slice(&[index; 32])
        .unwrap()
        .keypair(&Secp256k1::new())
        .x_only_public_key()
        .0
}

fn context(emulated: bool) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        if emulated {
            covenant::plan(4)
        } else {
            LoweringPlan::Native
        },
        "compatibility".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

struct Payment {
    action_guard: ScriptPolicy,
    template_guard: ScriptPolicy,
    version: i32,
    sequence: u32,
    lock_time: u32,
}

impl Payment {
    fn with_guard(guard: impl Into<ScriptPolicy>, on_template: bool) -> Self {
        let guard = guard.into();
        Self {
            action_guard: if on_template {
                Clause::Trivial.into()
            } else {
                guard.clone()
            },
            template_guard: if on_template {
                guard
            } else {
                Clause::Trivial.into()
            },
            version: 2,
            sequence: 10,
            lock_time: 100,
        }
    }

    #[guard(policy)]
    fn authorized(self, _ctx: Context) -> ScriptPolicy {
        self.action_guard.clone()
    }

    #[then(guarded_by = "[Self::authorized]")]
    fn pay(self, ctx: Context) {
        let mut template: Template = ctx
            .template()
            .add_output(Amount::from_sat(1_000), &key(9), None)?
            .add_guard(self.template_guard.clone())
            .into();
        template.tx.version = bitcoin::transaction::Version(self.version);
        template.tx.lock_time = bitcoin::absolute::LockTime::from_consensus(self.lock_time);
        template.tx.input[0].sequence = bitcoin::Sequence(self.sequence);
        template.ctv = template.tx.get_ctv_hash(0);
        Ok(Box::new(std::iter::once(Ok(template))))
    }
}

impl Contract for Payment {
    declare! {then, Self::pay}
    declare! {non updatable}
}

fn impossible(payment: &Payment, emulated: bool) {
    let error = payment.compile(context(emulated)).unwrap_err();
    let CompilationError::ImpossibleTemplate { at, .. } = error else {
        panic!("{error:?}")
    };
    assert_eq!(String::from(at), "compatibility/@action/pay/@next");
}

#[test]
fn fixed_transaction_must_meet_action_and_template_timelocks() {
    for emulated in [false, true] {
        for on_template in [false, true] {
            for guard in [
                Clause::Older(sapio::miniscript::RelLockTime::from_consensus(10).unwrap()),
                Clause::After(sapio::miniscript::AbsLockTime::from_consensus(100).unwrap()),
            ] {
                let mut payment = Payment::with_guard(guard.clone(), on_template);
                payment
                    .compile(context(emulated))
                    .unwrap()
                    .validate()
                    .unwrap();
                match guard {
                    Clause::Older(_) => {
                        payment.sequence = 9;
                        impossible(&payment, emulated);
                        payment.sequence = (1 << 22) | 10;
                        impossible(&payment, emulated);
                        payment.sequence = 10;
                        payment.version = 1;
                        impossible(&payment, emulated);
                    }
                    Clause::After(_) => {
                        payment.lock_time = 99;
                        impossible(&payment, emulated);
                        payment.lock_time = 500_000_000;
                        impossible(&payment, emulated);
                        payment.lock_time = 100;
                        payment.sequence = u32::MAX;
                        impossible(&payment, emulated);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[test]
fn viable_alternatives_remain_available_without_requiring_every_timelock() {
    for emulated in [false, true] {
        for on_template in [false, true] {
            let children = vec![
                Clause::Older(sapio::miniscript::RelLockTime::from_consensus(11).unwrap()),
                Clause::Key(key(1)),
                Clause::After(sapio::miniscript::AbsLockTime::from_consensus(100).unwrap()),
            ];
            for policy in [
                Clause::Or(vec![
                    (1, Arc::new(children[0].clone())),
                    (1, Arc::new(children[1].clone())),
                ]),
                Clause::Thresh(
                    sapio::miniscript::Threshold::new(
                        2,
                        (children.clone()).into_iter().map(Arc::new).collect(),
                    )
                    .unwrap(),
                ),
            ] {
                Payment::with_guard(policy, on_template)
                    .compile(context(emulated))
                    .unwrap();
            }
            impossible(
                &Payment::with_guard(
                    Clause::Thresh(
                        sapio::miniscript::Threshold::new(
                            3,
                            (children).into_iter().map(Arc::new).collect(),
                        )
                        .unwrap(),
                    ),
                    on_template,
                ),
                emulated,
            );
        }
    }
}

#[test]
fn an_additional_covenant_cannot_require_a_different_transaction() {
    for emulated in [false, true] {
        for on_template in [false, true] {
            impossible(
                &Payment::with_guard(
                    Clause::TxTemplate(sha256::Hash::hash(b"different template")),
                    on_template,
                ),
                emulated,
            );
        }
    }
}

#[test]
fn emulatable_guards_preserve_transaction_feasibility_before_key_lowering() {
    let original = Payment::with_guard(Clause::Trivial, false)
        .compile(context(false))
        .unwrap();
    let hash = *original.ctv_to_tx.keys().next().unwrap();
    let different = sha256::Hash::hash(b"different wrapped template");
    for emulated in [false, true] {
        for on_template in [false, true] {
            let matching = Payment::with_guard(Emulatable(Ctv(hash)), on_template)
                .compile(context(emulated))
                .unwrap();
            assert_eq!(
                matching.ctv_to_tx.keys().copied().collect::<Vec<_>>(),
                vec![hash]
            );
            assert_eq!(
                matching.covenant_requirements.predicates,
                std::collections::BTreeSet::from([Ctv(hash)])
            );
            impossible(
                &Payment::with_guard(Emulatable(Ctv(different)), on_template),
                emulated,
            );

            // A transaction only needs one feasible authorization alternative.
            // Rejecting every false subpredicate would incorrectly reject this.
            let alternative = ScriptPolicy::Or(vec![
                Emulatable(Ctv(different)).into(),
                Clause::Key(key(1)).into(),
            ]);
            Payment::with_guard(alternative, on_template)
                .compile(context(emulated))
                .unwrap();
        }
    }
}
