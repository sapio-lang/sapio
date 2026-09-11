use bitcoin::bip32::Xpub;
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::{ProgramClient, ProgramClientError, ProgramError, ProgramOracle};
use miniscript::psbt::PsbtExt;
use sapio_base::program::EmulatedProgram;
use sapio_integration_tests::program_example::*;

fn contract() -> PaymentContract {
    PaymentContract::new(
        5_000,
        recipient(92),
        Xpub::from_priv(&Secp256k1::new(), &example_root()),
    )
}

#[test]
fn continuation_candidates_preserve_the_complete_fixed_program_policy() {
    let source = contract();
    let original = source.compile_candidates(&[]).unwrap();
    let with_effects = source
        .compile_candidates(&[
            PaymentCandidate {
                amount: 6_000,
                recipient_first: true,
            },
            PaymentCandidate {
                amount: 7_000,
                recipient_first: false,
            },
        ])
        .unwrap();
    assert_eq!(original.address, with_effects.address);
    assert_eq!(original.descriptor, with_effects.descriptor);
    assert_eq!(original.metadata, with_effects.metadata);
    assert_eq!(original.suggested_txs.len(), 1);
    assert_eq!(with_effects.suggested_txs.len(), 3);
    assert!(with_effects.ctv_to_tx.is_empty());
    assert!(with_effects.covenant_requirements.predicates.is_empty());
    assert!(!with_effects.requires_native_ctv());

    let restored: sapio::contract::Compiled =
        serde_json::from_value(serde_json::to_value(&with_effects).unwrap()).unwrap();
    restored.validate().unwrap();
    let program: EmulatedProgram =
        serde_json::from_value(restored.metadata.extra[PROGRAM_METADATA].clone()).unwrap();
    assert_eq!(&program, source.emulation());
    let candidates = bind_candidates(&restored).unwrap();
    assert_eq!(candidates.len(), 3);
    for candidate in candidates {
        assert_eq!(candidate.inputs[0].tap_key_sig, None);
        assert!(candidate.inputs[0].tap_script_sigs.is_empty());
        assert_eq!(
            candidate.inputs[0].tap_internal_key,
            Some(program.derive_public_key().unwrap())
        );
    }
}

#[test]
fn recipient_minimum_and_oracle_root_are_fixed_before_funding() {
    let source = contract();
    let original = source.compile_candidates(&[]).unwrap();
    let root = *source.emulation().root();
    for replacement in [
        PaymentContract::new(5_001, recipient(92), root),
        PaymentContract::new(5_000, recipient(93), root),
        PaymentContract::new(
            5_000,
            recipient(92),
            Xpub::from_priv(
                &Secp256k1::new(),
                &bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Regtest, &[95; 32]).unwrap(),
            ),
        ),
    ] {
        let changed = replacement.compile_candidates(&[]).unwrap();
        assert_ne!(changed.address, original.address);
        assert_ne!(changed.metadata, original.metadata);
    }
}

#[test]
fn the_oracle_enforces_the_program_and_rejects_altered_authorization() {
    let source = contract();
    let oracle = ProgramOracle::new(example_root(), vec![pay_at_least_evaluator()]).unwrap();
    let compiled = source
        .compile_candidates(&[PaymentCandidate {
            amount: 4_999,
            recipient_first: true,
        }])
        .unwrap();
    let mut candidates = bind_candidates(&compiled).unwrap();
    let underpaid = candidates
        .iter()
        .position(|candidate| candidate.unsigned_tx.output[0].value.to_sat() == 4_999)
        .unwrap();
    let underpaid = candidates.remove(underpaid);
    assert!(matches!(
        oracle.sign(signing_request(source.emulation(), underpaid, 0)),
        Err(ProgramError::Rejected)
    ));

    let candidate = candidates.pop().unwrap();
    let request = signing_request(source.emulation(), candidate.clone(), 0);
    let signed = oracle.sign(request.clone()).unwrap();
    let mut finalized = signed.clone();
    finalized.finalize_mut(&Secp256k1::new()).unwrap();
    assert!(!finalized.extract_tx().unwrap().input[0].witness.is_empty());

    let mut wrong_recipient = request.clone();
    wrong_recipient.psbt.0.unsigned_tx.output[0].script_pubkey = recipient(93).script_pubkey();
    assert!(matches!(
        oracle.sign(wrong_recipient),
        Err(ProgramError::Rejected)
    ));
    for index in [1, 2, u32::MAX] {
        assert!(matches!(
            oracle.sign(signing_request(
                source.emulation(),
                candidate.clone(),
                index
            )),
            Err(ProgramError::Rejected)
        ));
    }
    for witness in [vec![], vec![0; 3], vec![0; 5]] {
        let mut malformed = request.clone();
        malformed.witness = witness;
        assert!(matches!(
            oracle.sign(malformed),
            Err(ProgramError::Evaluation(_))
        ));
    }

    let mut changed_parameters = request.clone();
    changed_parameters.instance = instance(4_999, &recipient(92).script_pubkey());
    assert!(matches!(
        oracle.sign(changed_parameters),
        Err(ProgramError::KeyPathMismatch)
    ));
    let wrong_root = ProgramOracle::new(
        bitcoin::bip32::Xpriv::new_master(bitcoin::Network::Regtest, &[95; 32]).unwrap(),
        vec![pay_at_least_evaluator()],
    )
    .unwrap();
    assert!(matches!(
        wrong_root.sign(request),
        Err(ProgramError::KeyPathMismatch)
    ));

    // A valid authorization cannot be reused to change a signed payment.
    for changed_recipient in [false, true] {
        let mut altered = signed.clone();
        if changed_recipient {
            altered.unsigned_tx.output[0].script_pubkey = recipient(93).script_pubkey();
        } else {
            altered.unsigned_tx.output[0].value -= bitcoin::Amount::ONE_SAT;
        }
        assert!(altered.finalize_mut(&Secp256k1::new()).is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_fixed_continuation_accepts_larger_and_reordered_payments_over_tcp() {
    let source = contract();
    // Compilation and binding finish before the signing service is created.
    let compiled = source
        .compile_candidates(&[
            PaymentCandidate {
                amount: 6_000,
                recipient_first: true,
            },
            PaymentCandidate {
                amount: 7_000,
                recipient_first: false,
            },
        ])
        .unwrap();
    let candidates = bind_candidates(&compiled).unwrap();
    let oracle = ProgramOracle::new(example_root(), vec![pay_at_least_evaluator()]).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = ProgramClient::new(address, oracle.public_root()).unwrap();
    let server = tokio::spawn(oracle.serve(listener));
    assert!(matches!(
        client
            .sign(signing_request(
                source.emulation(),
                candidates[0].clone(),
                u32::MAX,
            ))
            .await,
        Err(ProgramClientError::Rejected(_))
    ));
    let mut accepted = Vec::new();
    for candidate in candidates {
        let index = candidate
            .unsigned_tx
            .output
            .iter()
            .position(|output| output.script_pubkey == recipient(92).script_pubkey())
            .unwrap();
        let value = candidate.unsigned_tx.output[index].value.to_sat();
        let original_txid = candidate.unsigned_tx.compute_txid();
        let mut signed = client
            .sign(signing_request(source.emulation(), candidate, index as u32))
            .await
            .unwrap();
        signed.finalize_mut(&Secp256k1::new()).unwrap();
        let transaction = signed.extract_tx().unwrap();
        assert_eq!(transaction.compute_txid(), original_txid);
        assert!(!transaction.input[0].witness.is_empty());
        accepted.push((value, index));
    }
    accepted.sort_unstable();
    assert_eq!(accepted, [(5_000, 0), (6_000, 0), (7_000, 1)]);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
