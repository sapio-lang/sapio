use super::*;
use sapio::contract::Compilable;
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use std::sync::Arc;
fn context(amount: u64) -> Context {
    Context::new(
        bitcoin::Network::Regtest,
        Amount::from_sat(amount),
        LoweringPlan::Native,
        EffectPath::try_from("example").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}
fn address() -> bitcoin::Address {
    "bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj"
        .parse()
        .unwrap()
}
fn key(byte: u8) -> bitcoin::XOnlyPublicKey {
    use bitcoin::secp256k1::{Keypair, SecretKey};
    Keypair::from_secret_key(
        &bitcoin::secp256k1::Secp256k1::new(),
        &SecretKey::from_slice(&[byte; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

fn pool(count: u8) -> PaymentPool {
    PaymentPool {
        members: (1..=count)
            .map(|i| (key(i), Amount::from_sat(1000).into()))
            .collect(),
        sequence: 0,
        sig_needed: false,
    }
}
fn update(amount: u64, fee: u64) -> DoTx {
    DoTx {
        payments: [(
            key(1),
            PaymentRequest {
                hex_sig: String::new(),
                fee: fee.into(),
                payments: [(address(), Amount::from_sat(amount).into())].into(),
            },
        )]
        .into(),
    }
}
#[test]
fn withdrawal_accounts_for_fees_and_omits_empty_change_pool() {
    let template = pool(1)
        .continue_do_tx(context(1000), update(900, 100))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(template.tx.output.len(), 1);
    assert_eq!(template.tx.output[0].value, 900);
    assert_eq!(template.max, Amount::from_sat(1000));
    let template = pool(1)
        .continue_do_tx(context(1000), update(500, 100))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        template
            .tx
            .output
            .iter()
            .map(|o| o.value)
            .collect::<Vec<_>>(),
        vec![400, 500]
    );
    assert_eq!(template.max, Amount::from_sat(1000));
}
#[test]
fn overspending_and_exhausted_sequences_return_errors() {
    for (amount, fee) in [(1001, 0), (900, 101), (1000, u64::MAX)] {
        assert!(pool(1)
            .continue_do_tx(context(1000), update(amount, fee))
            .is_err());
    }
    let mut exhausted = pool(2);
    exhausted.sequence = u64::MAX;
    assert!(exhausted.compile(context(2000)).is_err());
    assert!(pool(0).compile(context(0)).is_err());
}
#[test]
fn ejected_singletons_are_spendable_only_by_their_owner() {
    assert_eq!(pool(1).guard_sole_owner(context(1000)), Clause::Key(key(1)));
    assert_eq!(
        pool(2).guard_sole_owner(context(2000)),
        Clause::Unsatisfiable
    );
    let compiled = pool(2).compile(context(2000)).unwrap();
    compiled.validate().unwrap();
    let template = compiled.ctv_to_tx.values().next().unwrap();
    assert_eq!(template.outputs.len(), 2);
    for output in &template.outputs {
        assert_eq!(output.amount, Amount::from_sat(1000));
        assert!(output.contract.ctv_to_tx.is_empty());
        output.contract.validate().unwrap();
    }
}

#[test]
fn signed_request_authenticates_fee_sequence_payments_and_sender() {
    use bitcoin::secp256k1::{Keypair, SecretKey};
    let mut contract = pool(1);
    contract.sig_needed = true;
    let mut request = update(500, 100);
    let mut committed = Vec::new();
    committed.extend_from_slice(&0u64.to_le_bytes());
    committed.extend_from_slice(&100u64.to_le_bytes());
    committed.extend_from_slice(&500u64.to_le_bytes());
    committed.extend_from_slice(address().script_pubkey().as_bytes());
    let digest = sha256::Hash::hash(&committed);
    let message = Message::from_digest_slice(&digest[..]).unwrap();
    let secp = Secp256k1::new();
    let sign = |byte| {
        let pair = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).unwrap());
        secp.sign_schnorr_no_aux_rand(&message, &pair).to_string()
    };
    request.payments.get_mut(&key(1)).unwrap().hex_sig = sign(1);
    let copy = || serde_json::from_value::<DoTx>(serde_json::to_value(&request).unwrap()).unwrap();
    assert!(contract.continue_do_tx(context(1000), copy()).is_ok());
    let mut changed = copy();
    changed.payments.get_mut(&key(1)).unwrap().fee = 101.into();
    assert!(contract.continue_do_tx(context(1000), changed).is_err());
    let mut changed = copy();
    changed
        .payments
        .get_mut(&key(1))
        .unwrap()
        .payments
        .insert(address(), Amount::from_sat(501).into());
    assert!(contract.continue_do_tx(context(1000), changed).is_err());
    let mut changed = copy();
    changed.payments.get_mut(&key(1)).unwrap().hex_sig = sign(2);
    assert!(contract.continue_do_tx(context(1000), changed).is_err());
    contract.sequence = 1;
    assert!(contract.continue_do_tx(context(1000), copy()).is_err());
}

#[test]
fn unknown_participant_cannot_create_a_payment() {
    let mut request = update(1, 0);
    let payment = request.payments.remove(&key(1)).unwrap();
    request.payments.insert(key(2), payment);
    assert!(pool(1).continue_do_tx(context(1000), request).is_err());
}
