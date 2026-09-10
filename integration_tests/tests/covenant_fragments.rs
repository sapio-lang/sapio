use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::{ProgramError, ProgramOracle, ProgramSpendPath};
use sapio_integration_tests::fragment_example::{participant_key, Authorization, FragmentContract};
use sapio_integration_tests::program_example::{bind_candidates, example_root};

#[test]
fn compiled_fragment_contracts_authorize_both_paths_and_preserve_annexes() {
    let secp = Secp256k1::new();
    let root = example_root();
    let oracle = ProgramOracle::new(root, vec![]).unwrap();
    let participant = participant_key();
    for (mode, signer, script_path) in [
        (
            Authorization::InternalKey(participant.x_only_public_key().0),
            participant,
            true,
        ),
        (Authorization::KnownTweak, root.to_keypair(&secp), false),
    ] {
        let source = FragmentContract::new(mode, oracle.public_root());
        let baseline = source.compile_candidates(&[]).unwrap();
        let compiled = source.compile_candidates(&[7_000]).unwrap();
        assert_eq!(baseline.address, compiled.address);
        assert_eq!(baseline.descriptor, compiled.descriptor);
        let candidates = bind_candidates(&compiled).unwrap();
        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            if script_path {
                assert_eq!(
                    candidate.inputs[0].tap_internal_key,
                    Some(participant.x_only_public_key().0)
                );
            }
            let request = source
                .signing_request(candidate, &signer, Some(b"\x50fragments-test".to_vec()))
                .unwrap();
            assert_eq!(
                matches!(request.path, ProgramSpendPath::ScriptPath(_)),
                script_path
            );
            if !script_path {
                assert!(request.psbt.0.inputs[0].tap_internal_key.is_none());
                assert!(request.psbt.0.inputs[0].tap_scripts.is_empty());
            }
            let mut bad_evidence = request.clone();
            *bad_evidence.witness.last_mut().unwrap() ^= 1;
            assert!(matches!(
                oracle.sign(bad_evidence),
                Err(ProgramError::Evaluation(_))
            ));
            let mut bad_annex = request.clone();
            sapio_psbt::annex::set(
                &mut bad_annex.psbt.0.inputs[0],
                Some(b"\x50other-annex".to_vec()),
            )
            .unwrap();
            assert!(matches!(
                oracle.sign(bad_annex),
                Err(ProgramError::Evaluation(_))
            ));
            let signed = oracle.sign(request).unwrap();
            let finalized = sapio_psbt::finalize::finalize(signed, &secp).unwrap();
            let tx = finalized.extract_tx();
            assert_eq!(tx.input[0].witness.last(), Some(&b"\x50fragments-test"[..]));
            assert_eq!(tx.input[0].witness.len(), if script_path { 4 } else { 2 });
        }
    }
}
