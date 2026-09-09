use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::{Compilable, CompilationError, Contract};
use sapio::template::Template;
use sapio::{declare, guard, then, Context};
use sapio_base::{CTVHash, Clause};
use sapio_ctv_emulator_trait::{CTVAvailable, CTVEmulator, EmulatorError};
use std::sync::Arc;

fn key(index: u8) -> XOnlyPublicKey {
    SecretKey::from_slice(&[index; 32])
        .unwrap()
        .keypair(&Secp256k1::new())
        .x_only_public_key()
        .0
}

struct Emulated;
impl CTVEmulator for Emulated {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::Key(key(4)))
    }
    fn sign(
        &self,
        _: bitcoin::util::psbt::PartiallySignedTransaction,
    ) -> Result<bitcoin::util::psbt::PartiallySignedTransaction, EmulatorError> {
        unreachable!("compilation must not sign")
    }
}

fn context(emulated: bool) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(1_000),
        if emulated {
            Arc::new(Emulated)
        } else {
            Arc::new(CTVAvailable)
        },
        "compatibility".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

struct Payment {
    action_guard: Clause,
    template_guard: Clause,
    version: i32,
    sequence: u32,
    lock_time: u32,
}

impl Payment {
    fn with_guard(guard: Clause, on_template: bool) -> Self {
        Self {
            action_guard: if on_template {
                Clause::Trivial
            } else {
                guard.clone()
            },
            template_guard: if on_template { guard } else { Clause::Trivial },
            version: 2,
            sequence: 10,
            lock_time: 100,
        }
    }

    #[guard]
    fn authorized(self, _ctx: Context) {
        self.action_guard.clone()
    }

    #[then(guarded_by = "[Self::authorized]")]
    fn pay(self, ctx: Context) {
        let mut template: Template = ctx
            .template()
            .add_output(Amount::from_sat(1_000), &key(9), None)?
            .add_guard(self.template_guard.clone())
            .into();
        template.tx.version = self.version;
        template.tx.lock_time = self.lock_time;
        template.tx.input[0].sequence = self.sequence;
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
            for guard in [Clause::Older(10), Clause::After(100)] {
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
            let children = vec![Clause::Older(11), Clause::Key(key(1)), Clause::After(100)];
            for policy in [
                Clause::Or(vec![(1, children[0].clone()), (1, children[1].clone())]),
                Clause::Threshold(2, children.clone()),
            ] {
                Payment::with_guard(policy, on_template)
                    .compile(context(emulated))
                    .unwrap();
            }
            impossible(
                &Payment::with_guard(Clause::Threshold(3, children), on_template),
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
