use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use sapio::contract::actions::ThenFuncAsFinishOrFunc;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::{Compilable, CompilationError, Contract};
use sapio::miniscript::policy::{semantic::Policy, Liftable};
use sapio::miniscript::MiniscriptKey;
use sapio::template::Template;
use sapio::{continuation, declare, guard, then, Context};
use sapio_base::Clause;
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
        Amount::from_sat(2_000),
        if emulated {
            Arc::new(Emulated)
        } else {
            Arc::new(CTVAvailable)
        },
        "payments".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn payment(ctx: Context, guards: &[Clause]) -> Result<Template, CompilationError> {
    let mut builder = ctx
        .template()
        .add_output(Amount::from_sat(1_000), &key(9), None)?;
    for guard in guards {
        builder = builder.add_guard(guard.clone());
    }
    Ok(builder.into())
}

struct Payments<const REVERSED: bool>;
impl<const REVERSED: bool> Payments<REVERSED> {
    #[guard]
    fn alice(self, _ctx: Context) {
        Clause::Key(key(1))
    }
    #[guard]
    fn bob(self, _ctx: Context) {
        Clause::Key(key(2))
    }
    #[then(guarded_by = "[Self::alice]")]
    fn alice_payment(self, ctx: Context) {
        Ok(Box::new(std::iter::once(payment(ctx, &[]))))
    }
    #[then(guarded_by = "[Self::bob]")]
    fn bob_carol_payment(self, ctx: Context) {
        Ok(Box::new(std::iter::once(payment(
            ctx,
            &[Clause::Key(key(3))],
        ))))
    }
}
impl<const REVERSED: bool> Contract for Payments<REVERSED> {
    const THEN_FNS: &'static [fn() -> Option<ThenFuncAsFinishOrFunc<'static, Self, ()>>] =
        if REVERSED {
            &[Self::bob_carol_payment, Self::alice_payment]
        } else {
            &[Self::alice_payment, Self::bob_carol_payment]
        };
    declare! {non updatable}
}

fn accepts(policy: &Policy<XOnlyPublicKey>, mask: u8, commitment: sha256::Hash) -> bool {
    match policy {
        Policy::Unsatisfiable => false,
        Policy::Trivial => true,
        Policy::KeyHash(hash) => (1..=4)
            .any(|index| mask & (1 << (index - 1)) != 0 && key(index).to_pubkeyhash() == *hash),
        Policy::Threshold(required, children) => {
            children
                .iter()
                .filter(|child| accepts(child, mask, commitment))
                .count()
                >= *required
        }
        Policy::TxTemplate(hash) => *hash == commitment,
        _ => panic!("unexpected policy in authorization fixture"),
    }
}

#[test]
fn duplicate_transactions_preserve_every_authorization_and_its_action_guard() {
    for emulated in [false, true] {
        let forward = Payments::<false>.compile(context(emulated)).unwrap();
        let reverse = Payments::<true>.compile(context(emulated)).unwrap();
        assert_eq!(forward.descriptor, reverse.descriptor);
        assert_eq!(forward.ctv_to_tx, reverse.ctv_to_tx);
        assert_eq!(forward.ctv_to_tx.len(), 1);
        forward.validate().unwrap();
        let template = forward.ctv_to_tx.values().next().unwrap();
        let Some(SupportedDescriptors::XOnly(descriptor)) = &forward.descriptor else {
            panic!()
        };
        let compiled_policy = descriptor.lift().unwrap();
        assert_eq!(template.guards.len(), 1);
        let stored_guards = template.guards[0].lift().unwrap();
        for mask in 0..16 {
            let authorized = mask & 1 != 0 || mask & 6 == 6;
            assert_eq!(accepts(&stored_guards, mask, template.hash()), authorized);
            for hash in [template.hash(), sha256::Hash::hash(b"another transaction")] {
                let covenant = if emulated {
                    mask & 8 != 0
                } else {
                    hash == template.hash()
                };
                assert_eq!(
                    accepts(&compiled_policy, mask, hash),
                    authorized && covenant,
                    "emulated={emulated}, signers={mask}, hash={hash}"
                );
            }
        }
    }
}

struct RepeatedTemplates {
    alter: fn(&mut Template),
}
impl RepeatedTemplates {
    #[then]
    fn pay(self, ctx: Context) {
        let first = payment(ctx, &[])?;
        let mut second = first.clone();
        (self.alter)(&mut second);
        Ok(Box::new([Ok(first), Ok(second)].into_iter()))
    }
}
impl Contract for RepeatedTemplates {
    declare! {then, Self::pay}
    declare! {non updatable}
}

#[test]
fn duplicate_commitments_reject_conflicting_binding_payloads() {
    type Alteration = fn(&mut Template);
    let alterations: [(Alteration, &str); 6] = [
        (|t| t.max += Amount::from_sat(1), "max"),
        (
            |t| t.required_input_amount += Amount::from_sat(1),
            "required_input_amount",
        ),
        (
            |t| t.min_feerate_sats_vbyte = Some(Amount::from_sat(1)),
            "min_feerate_sats_vbyte",
        ),
        (
            |t| t.metadata_map_s2s.label = Some("different label".into()),
            "metadata_map_s2s",
        ),
        (
            |t| {
                t.inputs[0].extra.insert("input".into(), true.into());
            },
            "inputs",
        ),
        (
            |t| {
                t.outputs[0].contract.root_path = sapio_base::serialization_helpers::SArc(Arc::new(
                    "different_continuation".try_into().unwrap(),
                ))
            },
            "outputs",
        ),
    ];
    for (alter, expected_field) in alterations {
        let error = RepeatedTemplates { alter }
            .compile(context(false))
            .unwrap_err();
        let CompilationError::ConflictingTemplate { at, field, .. } = error else {
            panic!("{error:?}")
        };
        assert_eq!(field, expected_field);
        assert_eq!(String::from(at), "payments/@action/pay/@next");
    }
}

struct Suggested;
impl Suggested {
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(key(1))
    }
    #[continuation(guarded_by = "[Self::signed]", coerce_args = "Ok")]
    fn update(self, ctx: Context, _args: ()) {
        let first = payment(ctx, &[])?;
        let mut second = first.clone();
        second.guards.push(Clause::Key(key(2)));
        Ok(Box::new([Ok(first), Ok(second)].into_iter()))
    }
}
impl Contract for Suggested {
    declare! {updatable<()>, Self::update}
}

#[test]
fn suggested_template_duplicates_cannot_hide_forbidden_guards() {
    assert!(matches!(
        Suggested.compile(context(false)),
        Err(CompilationError::AdditionalGuardsNotAllowedHere)
    ));
}

struct ManyGuards;
impl ManyGuards {
    #[then]
    fn pay(self, ctx: Context) {
        Ok(Box::new(std::iter::once(payment(
            ctx,
            &[
                Clause::Key(key(1)),
                Clause::Key(key(2)),
                Clause::Key(key(3)),
            ],
        ))))
    }
}
impl Contract for ManyGuards {
    declare! {then, Self::pay}
    declare! {non updatable}
}

#[test]
fn multiple_template_guards_all_remain_required() {
    let compiled = ManyGuards.compile(context(false)).unwrap();
    let Some(SupportedDescriptors::XOnly(descriptor)) = &compiled.descriptor else {
        panic!()
    };
    let policy = descriptor.lift().unwrap();
    let hash = compiled.ctv_to_tx.keys().next().copied().unwrap();
    for mask in 0..8 {
        assert_eq!(accepts(&policy, mask, hash), mask == 7);
    }
}
