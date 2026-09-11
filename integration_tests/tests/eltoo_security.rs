use bitcoin::consensus::{deserialize, serialize};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{schnorr::Signature, Secp256k1};
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{taproot, ScriptBuf, Transaction};
use emulator_connect::program::{
    validate_program_response, ProgramError, ProgramOracle, ProgramSigningRequest,
    ProgramSpendPath, PSBT,
};
use sapio_base::fragments::template_hash;
use sapio_integration_tests::eltoo_example::{
    attach_inputs, authorize_update, fixture, recovery::recover_update, settlement_request,
    sign_sponsor, update_request, Channel, State, Terms,
};

fn state(number: u32) -> State {
    State {
        number,
        alice_sats: 60_000,
    }
}

fn oracle() -> ProgramOracle {
    ProgramOracle::new(fixture::oracle_key(), vec![]).unwrap()
}

fn sign(oracle: &ProgramOracle, request: ProgramSigningRequest) -> Psbt {
    let expected = request.clone();
    let mut signed = oracle.sign(request).unwrap();
    validate_program_response(&expected, &signed, &oracle.public_root()).unwrap();
    sign_sponsor(&mut signed, &fixture::sponsor_key()).unwrap();
    signed
}

fn finish(oracle: &ProgramOracle, request: ProgramSigningRequest) -> Transaction {
    sapio_psbt::finalize::finalize(sign(oracle, request), &Secp256k1::new())
        .unwrap()
        .extract_tx()
        .unwrap()
}

// Bypass only the candidate builder's monotonicity check, allowing tests to
// exercise the native script guard with an otherwise authentic certificate.
fn raw_update_request(
    source: &Channel,
    transaction: Transaction,
    authorization: &Signature,
) -> ProgramSigningRequest {
    let coin = fixture::coin(source, 80);
    let input = source.input(&coin).unwrap();
    let leaf = source.update_leaf().unwrap();
    ProgramSigningRequest {
        instance: source.terms().update_program().instance().clone(),
        input_index: 0,
        witness: authorization.as_ref().to_vec(),
        path: ProgramSpendPath::ScriptPath(TapLeafHash::from_script(&leaf, LeafVersion::TapScript)),
        psbt: PSBT(attach_inputs(transaction, coin, input, fixture::sponsor(2_000, 81)).unwrap()),
    }
}

#[test]
fn funding_and_maximum_state_have_no_unintended_escape_leaf() {
    let terms = fixture::terms();
    let funding = terms.funding();
    let funding_coin = fixture::coin(&funding, 10);
    let input = funding.input(&funding_coin).unwrap();
    assert_eq!(input.tap_internal_key, Some(terms.joint_key()));
    assert_eq!(input.tap_scripts.len(), 1);
    assert_eq!(
        input.tap_scripts.values().next().unwrap().0,
        funding.update_leaf().unwrap()
    );
    assert!(funding.settlement_program().is_none());
    assert!(funding.compile_settlement().is_err());
    assert!(settlement_request(&funding, funding_coin, fixture::sponsor(2_000, 11)).is_err());

    let last = terms.state(state(terms.max_state())).unwrap();
    let last_input = last.input(&fixture::coin(&last, 12)).unwrap();
    assert_eq!(last_input.tap_internal_key, Some(terms.joint_key()));
    assert_eq!(last_input.tap_scripts.len(), 1);
    assert_eq!(
        last_input.tap_scripts.values().next().unwrap().0,
        last.settlement_leaf().unwrap()
    );
    assert!(last.update_leaf().is_err());
    assert!(last.compile_update(state(terms.max_state())).is_err());
    assert!(terms.state(state(0)).is_err());
    assert!(terms.state(state(terms.max_state() + 1)).is_err());
    assert!(authorize_update(&terms, state(terms.max_state() + 1), &fixture::joint_key()).is_err());

    // An exhausted state retains its delayed exit. Actual chain age remains
    // a consensus check, exercised by the separate Core driver.
    let request =
        settlement_request(&last, fixture::coin(&last, 13), fixture::sponsor(2_000, 14)).unwrap();
    let finalized = finish(&oracle(), request);
    assert_eq!(
        finalized.input[0].sequence.to_consensus_u32(),
        u32::from(terms.delay())
    );
}

#[test]
fn repeated_payouts_do_not_allow_old_settlement_replay() {
    let terms = fixture::terms();
    let old = terms.state(state(1)).unwrap();
    let current = terms.state(state(2)).unwrap();
    let old_template = terms.settlement_transaction(state(1)).unwrap();
    let current_template = terms.settlement_transaction(state(2)).unwrap();
    assert_eq!(old_template.output, current_template.output);
    assert_ne!(old_template.lock_time, current_template.lock_time);
    assert_ne!(
        old.settlement_program().unwrap().instance().id(),
        current.settlement_program().unwrap().instance().id()
    );

    let oracle = oracle();
    let valid = settlement_request(
        &current,
        fixture::coin(&current, 20),
        fixture::sponsor(2_000, 21),
    )
    .unwrap();
    finish(&oracle, valid.clone());
    let mut replay = valid;
    replay.psbt.0.unsigned_tx.lock_time = old_template.lock_time;
    assert_eq!(
        template_hash(&replay.psbt.0.unsigned_tx, 0, None).unwrap(),
        template_hash(&old_template, 0, None).unwrap()
    );
    assert!(matches!(oracle.sign(replay), Err(ProgramError::Rejected)));
}

#[test]
fn one_sided_balances_settle_the_full_capacity_without_zero_outputs() {
    let terms = fixture::terms();
    let oracle = oracle();
    for (alice_sats, recipient, tag) in
        [(0, terms.bob(), 22), (terms.capacity(), terms.alice(), 24)]
    {
        let state = State {
            number: 2,
            alice_sats,
        };
        let channel = terms.state(state).unwrap();
        let transaction = finish(
            &oracle,
            settlement_request(
                &channel,
                fixture::coin(&channel, tag),
                fixture::sponsor(2_000, tag + 1),
            )
            .unwrap(),
        );
        assert_eq!(transaction.output.len(), 1);
        assert_eq!(transaction.output[0].value.to_sat(), terms.capacity());
        assert_eq!(
            transaction.output[0].script_pubkey,
            recipient.script_pubkey()
        );
        assert_eq!(
            transaction.output,
            terms.settlement_transaction(state).unwrap().output
        );
    }
    assert!(terms
        .state(State {
            number: 2,
            alice_sats: terms.capacity() + 1,
        })
        .is_err());
}

#[test]
fn unchanged_update_certificate_cannot_bypass_native_state_ordering() {
    let terms = fixture::terms();
    let certificate = authorize_update(&terms, state(1), &fixture::joint_key()).unwrap();
    let template = terms.update_transaction(state(1)).unwrap();
    let oracle = oracle();
    for number in [1, 2] {
        let source = terms.state(state(number)).unwrap();
        assert!(source.compile_update(state(1)).is_err());
        let request = raw_update_request(&source, template.clone(), &certificate);
        // The certificate remains valid. Native CLTV independently rejects
        // both equality and regression even after fresh ALL signatures.
        let signed = sign(&oracle, request);
        assert!(sapio_psbt::finalize::finalize(signed, &Secp256k1::new()).is_err());
    }
}

#[test]
fn update_evidence_does_not_authorize_the_cooperative_or_settlement_path() {
    let terms = fixture::terms();
    let source = terms.state(state(1)).unwrap();
    let certificate = authorize_update(&terms, state(3), &fixture::joint_key()).unwrap();
    let request = update_request(
        &source,
        state(3),
        fixture::coin(&source, 30),
        fixture::sponsor(2_000, 31),
        &certificate,
    )
    .unwrap();
    let oracle = oracle();
    let mut key_path = request.clone();
    key_path.path = ProgramSpendPath::KeyPath;
    assert!(matches!(
        oracle.sign(key_path),
        Err(ProgramError::KeyPathMismatch)
    ));

    let mut other_leaf = request;
    let settlement_script = source.settlement_leaf().unwrap();
    other_leaf.path = ProgramSpendPath::ScriptPath(TapLeafHash::from_script(
        &settlement_script,
        LeafVersion::TapScript,
    ));
    other_leaf.psbt.0.inputs[0]
        .tap_scripts
        .retain(|_, (script, _)| script == &settlement_script);
    // A program signature certifies its predicate; it cannot satisfy a
    // different program's leaf or remove that leaf's native CSV constraint.
    let signed = sign(&oracle, other_leaf);
    assert!(sapio_psbt::finalize::finalize(signed, &Secp256k1::new()).is_err());
}

#[test]
fn certificate_rebinding_preserves_capacity_and_accepts_replaceable_fee_coins() {
    let terms = fixture::terms();
    let source = terms.funding();
    let certificate = authorize_update(&terms, state(3), &fixture::joint_key()).unwrap();
    let expected = template_hash(&terms.update_transaction(state(3)).unwrap(), 0, None).unwrap();
    let oracle = oracle();
    let mut transactions = Vec::new();
    for (fee, tag) in [(2_000, 40), (5_000, 41)] {
        let request = update_request(
            &source,
            state(3),
            fixture::coin(&source, 42),
            fixture::sponsor(fee, tag),
            &certificate,
        )
        .unwrap();
        assert_eq!(
            template_hash(&request.psbt.0.unsigned_tx, 0, None).unwrap(),
            expected
        );
        assert_eq!(request.witness, certificate.as_ref());
        assert_eq!(
            request.psbt.0.inputs[0]
                .witness_utxo
                .as_ref()
                .unwrap()
                .value
                .to_sat(),
            terms.capacity()
        );
        assert_eq!(
            request.psbt.0.inputs[1]
                .witness_utxo
                .as_ref()
                .unwrap()
                .value
                .to_sat(),
            fee
        );
        let transaction = finish(&oracle, request);
        assert_eq!(
            transaction
                .output
                .iter()
                .map(|output| output.value.to_sat())
                .sum::<u64>(),
            terms.capacity()
        );
        transactions.push(transaction);
    }
    assert_ne!(
        transactions[0].compute_txid(),
        transactions[1].compute_txid()
    );
    assert_ne!(
        transactions[0].input[0].witness,
        transactions[1].input[0].witness
    );
    assert_ne!(
        transactions[0].input[1].witness,
        transactions[1].input[1].witness
    );
    let mut reused_signature = update_request(
        &source,
        state(3),
        fixture::coin(&source, 42),
        fixture::sponsor(5_000, 41),
        &certificate,
    )
    .unwrap();
    let ProgramSpendPath::ScriptPath(leaf) = reused_signature.path else {
        panic!("update must use its guarded script path");
    };
    reused_signature.psbt.0.inputs[0].tap_script_sigs.insert(
        (terms.update_program().derive_public_key().unwrap(), leaf),
        taproot::Signature::from_slice(transactions[0].input[0].witness.iter().next().unwrap())
            .unwrap(),
    );
    assert!(matches!(
        oracle.sign(reused_signature),
        Err(ProgramError::ConflictingSignature)
    ));

    for value in [terms.capacity() - 1, terms.capacity() + 1] {
        let mut wrong_amount = fixture::coin(&source, 43);
        wrong_amount.txout.value = bitcoin::Amount::from_sat(value);
        assert!(source.input(&wrong_amount).is_err());
    }
}

#[test]
fn fresh_channel_internal_key_prevents_cross_channel_certificate_replay() {
    let terms = fixture::terms();
    let certificate = authorize_update(&terms, state(3), &fixture::joint_key()).unwrap();
    let other_terms = Terms::new(
        fixture::sponsor_key().x_only_public_key().0,
        *terms.update_program().root(),
        terms.capacity(),
        terms.delay(),
        terms.max_state(),
        terms.alice().clone(),
        terms.bob().clone(),
    )
    .unwrap();
    assert!(authorize_update(&other_terms, state(3), &fixture::joint_key()).is_err());
    let template = terms.update_transaction(state(3)).unwrap();
    let expected = template_hash(&template, 0, None).unwrap();
    // Preserve channel A's entire authorized transaction while rebinding its
    // input to B. Failure must come from B's authenticated IKEY, not outputs.
    let request = raw_update_request(&other_terms.funding(), template, &certificate);
    assert_eq!(
        template_hash(&request.psbt.0.unsigned_tx, 0, None).unwrap(),
        expected
    );
    assert!(matches!(
        oracle().sign(request),
        Err(ProgramError::Evaluation(_))
    ));
}

#[test]
fn latest_certificate_recovers_an_old_state_without_old_payout_metadata() {
    let terms = fixture::terms();
    let oracle = oracle();
    let observed_bytes = {
        let old = State {
            number: 1,
            // Recovery's dummy allocation is 50/50, so it cannot accidentally
            // reconstruct the correct tree without the published leaf hash.
            alice_sats: 37_000,
        };
        let authorization = authorize_update(&terms, old, &fixture::joint_key()).unwrap();
        let funding = terms.funding();
        let request = update_request(
            &funding,
            old,
            fixture::coin(&funding, 50),
            fixture::sponsor(2_000, 51),
            &authorization,
        )
        .unwrap();
        serialize(&finish(&oracle, request))
    };
    // All old source objects and evidence have left scope. Only chain bytes,
    // fixed terms and the latest off-chain agreement are needed below.
    let observed: Transaction = deserialize(&observed_bytes).unwrap();
    let recovered = recover_update(&terms, &observed).unwrap();
    assert_eq!(recovered.state_number, 1);
    assert_eq!(recovered.coin.outpoint.txid, observed.compute_txid());
    let latest = state(3);
    let authorization = authorize_update(&terms, latest, &fixture::joint_key()).unwrap();
    assert!(recovered
        .request(
            &terms,
            state(1),
            fixture::sponsor(5_000, 52),
            &authorization
        )
        .is_err());
    let request = recovered
        .request(&terms, latest, fixture::sponsor(5_000, 52), &authorization)
        .unwrap();
    let transaction = finish(&oracle, request);
    assert_eq!(
        transaction.input[0].previous_output.txid,
        observed.compute_txid()
    );
    assert_eq!(
        transaction.output[0].script_pubkey,
        ScriptBuf::from(&terms.state(latest).unwrap().compile().unwrap().address)
    );
    assert_eq!(transaction.output[0].value.to_sat(), terms.capacity());
}
