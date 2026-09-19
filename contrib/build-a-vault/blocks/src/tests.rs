use super::*;
use bitcoin::opcodes::all::OP_NOP4;
use bitcoin::script::Instruction;
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
use bitcoin::{Amount, ScriptBuf};
use sapio::contract::object::SupportedDescriptors;
use sapio_base::covenant::LoweringPlan;
use sapio_base::effects::EffectPath;
use sapio_base::miniscript::policy::Liftable;
use sapio_base::miniscript::Descriptor;
use std::sync::Arc;

fn context(amount: u64) -> Context {
    Context::new(
        Network::Regtest,
        Amount::from_sat(amount),
        LoweringPlan::Native,
        EffectPath::try_from("buildavault").unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

fn key(seed: u8) -> XOnlyPublicKey {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
    .x_only_public_key()
    .0
}

fn signer(seed: u8) -> KeySet {
    Signer { key: key(seed) }.call(context(100_000)).unwrap()
}

fn destination(seed: u8) -> AddressTarget {
    Destination {
        address: Address::p2tr(&Secp256k1::new(), key(seed), None, Network::Regtest)
            .into_unchecked(),
    }
    .call(context(100_000))
    .unwrap()
}

fn vault() -> FixedVault {
    FixedVault {
        trigger: signer(1),
        release: Release {
            authorization: signer(2),
            delay: BlockDelay { blocks: 144 }.call(context(100_000)).unwrap(),
            destination: destination(3),
        }
        .call(context(100_000))
        .unwrap(),
        recovery: Recovery {
            authorization: Quorum {
                threshold: 2,
                keys: vec![key(4), key(5), key(6)],
            }
            .call(context(100_000))
            .unwrap(),
            destination: destination(7),
        }
        .call(context(100_000))
        .unwrap(),
        fee_sats: 500,
    }
}

#[test]
fn signers_and_quorums_produce_the_same_wire_type() {
    let one = signer(1);
    let quorum = Quorum {
        threshold: 1,
        keys: vec![key(1)],
    }
    .call(context(1))
    .unwrap();
    assert_eq!(one, quorum);
    assert_eq!(one.clause().unwrap(), Clause::Key(key(1)));
    let keys = vec![key(1), key(2), key(3)];
    for threshold in 1..=3 {
        assert!(Quorum {
            threshold,
            keys: keys.clone()
        }
        .call(context(1))
        .is_ok());
    }
}

#[test]
fn canonical_quorums_preserve_threshold_and_delay_semantics_through_sixteen_keys() {
    let all_keys: Vec<_> = (1..=16).map(key).collect();
    for count in 2..=16 {
        for threshold in 1..=count {
            let keys = KeySet {
                threshold: threshold as u8,
                keys: all_keys[..count].to_vec(),
            };
            let ScriptPolicy::Script(fragment) = keys.policy().unwrap() else {
                panic!("expected canonical quorum");
            };
            let script =
                Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(fragment.as_script()).unwrap();
            assert_eq!(
                script.lift().unwrap(),
                keys.clause().unwrap().lift().unwrap()
            );
            let delay = RelativeDelay { blocks: 144 };
            let ScriptPolicy::Script(fragment) = keys.delayed_policy(delay).unwrap() else {
                panic!("expected delayed canonical quorum");
            };
            let delayed =
                Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(fragment.as_script()).unwrap();
            let expected = Clause::And(vec![
                keys.clause().unwrap().into(),
                Clause::try_from(delay.timelock().unwrap()).unwrap().into(),
            ]);
            assert_eq!(delayed.lift().unwrap(), expected.lift().unwrap());
        }
    }
}

#[test]
fn canonical_quorums_finalize_with_the_required_native_signatures() {
    use bitcoin::bip32::Xpriv;
    use bitcoin::psbt::Psbt;
    use bitcoin::taproot::TapLeafHash;
    use sapio_psbt::selected::SignatureSlot;
    use sapio_psbt::SigningKey;

    let secp = Secp256k1::new();
    let roots: Vec<_> = (1..=16)
        .map(|seed| Xpriv::new_master(Network::Regtest, &[seed; 32]).unwrap())
        .collect();
    for (count, threshold) in [(3, 2), (16, 8)] {
        let authorization = KeySet {
            threshold,
            keys: roots[..count]
                .iter()
                .map(|root| root.to_keypair(&secp).x_only_public_key().0)
                .collect(),
        };
        let mut terms = vault();
        terms.trigger = authorization.clone();
        terms.release.authorization = authorization.clone();
        let compiled = terms.call(context(100_000)).unwrap();
        let pending = &named_template(&compiled, "pending").outputs[0].contract;
        let wallet = DelayedWallet {
            hot: authorization.clone(),
            delay: RelativeDelay { blocks: 144 },
            recovery: signer(31),
        }
        .call(context(100_000))
        .unwrap();
        let wallet_tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                sequence: bitcoin::Sequence(144),
                ..Default::default()
            }],
            output: vec![bitcoin::TxOut {
                value: Amount::from_sat(99_000),
                script_pubkey: destination(32)
                    .checked(Network::Regtest)
                    .unwrap()
                    .script_pubkey(),
            }],
        };
        for (object, transaction) in [
            (&compiled, named_template(&compiled, "pending").tx.clone()),
            (pending, named_template(pending, "release").tx.clone()),
            (&wallet, wallet_tx),
        ] {
            let mut psbt = Psbt::from_unsigned_tx(transaction).unwrap();
            psbt.inputs[0].witness_utxo = Some(bitcoin::TxOut {
                value: object.required_input_amount,
                script_pubkey: ScriptBuf::from(&object.address),
            });
            object
                .descriptor
                .as_ref()
                .unwrap()
                .update_psbt_input(&mut psbt.inputs[0])
                .unwrap();
            let slots: Vec<_> = psbt.inputs[0]
                .tap_scripts
                .values()
                .flat_map(|(script, version)| {
                    let leaf = Some(TapLeafHash::from_script(script, *version));
                    authorization
                        .keys
                        .iter()
                        .map(move |key| SignatureSlot::Schnorr { key: *key, leaf })
                })
                .collect();
            SigningKey(roots[..usize::from(threshold) - 1].to_vec())
                .sign_selected_input_mut(&mut psbt, &secp, 0, &slots)
                .unwrap();
            assert!(sapio_psbt::finalize::finalize(psbt.clone(), &secp).is_err());
            SigningKey(roots[..usize::from(threshold)].to_vec())
                .sign_selected_input_mut(&mut psbt, &secp, 0, &slots)
                .unwrap();
            let finalized = sapio_psbt::finalize::finalize(psbt, &secp).unwrap();
            assert!(finalized.extract_tx().unwrap().input[0].witness.len() >= count + 2);
        }
    }
}

#[test]
fn every_keyset_consumer_checks_thresholds_and_distinct_keys() {
    for keys in [
        KeySet {
            threshold: 0,
            keys: vec![key(1)],
        },
        KeySet {
            threshold: 2,
            keys: vec![key(1)],
        },
        KeySet {
            threshold: 1,
            keys: vec![],
        },
        KeySet {
            threshold: 1,
            keys: vec![key(1), key(1)],
        },
        KeySet {
            threshold: 1,
            keys: (1..=17).map(key).collect(),
        },
    ] {
        assert!(keys.clause().is_err());
        assert!(Quorum {
            threshold: keys.threshold,
            keys: keys.keys.clone()
        }
        .call(context(1))
        .is_err());
        let mut terms = vault();
        terms.trigger = keys.clone();
        assert!(terms.call(context(100_000)).is_err());
        terms = vault();
        terms.release.authorization = keys.clone();
        assert!(terms.call(context(100_000)).is_err());
        terms = vault();
        terms.recovery.authorization = keys;
        assert!(terms.call(context(100_000)).is_err());
    }
}

#[test]
fn block_delay_rejects_zero_and_accepts_the_consensus_boundary() {
    assert!(BlockDelay { blocks: 0 }.call(context(1)).is_err());
    assert!(serde_json::from_str::<BlockDelay>(r#"{"blocks":65536}"#).is_err());
    for blocks in [1, u16::MAX] {
        assert_eq!(
            BlockDelay { blocks }.call(context(1)).unwrap().blocks,
            blocks
        );
    }
    let mut terms = vault();
    terms.release.delay.blocks = 0;
    assert!(terms.call(context(100_000)).is_err());
}

#[test]
fn destination_network_is_rechecked_after_direct_json_input() {
    let mainnet = Address::p2tr(&Secp256k1::new(), key(1), None, Network::Bitcoin).into_unchecked();
    assert!(Destination {
        address: mainnet.clone()
    }
    .call(context(1))
    .is_err());
    let mut terms = vault();
    terms.recovery.destination.address = mainnet.clone();
    assert!(terms.call(context(100_000)).is_err());
    terms = vault();
    terms.release.destination.address = mainnet;
    assert!(terms.call(context(100_000)).is_err());
}

#[test]
fn dto_and_module_inputs_reject_misspelled_fields() {
    let mut value = serde_json::to_value(signer(1)).unwrap();
    value["threshhold"] = 1.into();
    assert!(serde_json::from_value::<KeySet>(value).is_err());
    let mut value = serde_json::to_value(vault()).unwrap();
    value["fees_sats"] = 1000.into();
    assert!(serde_json::from_value::<FixedVault>(value).is_err());
}

fn named_template<'a>(object: &'a Compiled, name: &str) -> &'a sapio::template::Template {
    object
        .ctv_to_tx
        .values()
        .find(|template| template.funding_constraints.as_ref().unwrap().outputs == [name])
        .unwrap()
}

fn assert_guarded_templates_only(object: &Compiled, authorities: &[XOnlyPublicKey]) {
    let (internal, scripts) = match object.descriptor.as_ref().unwrap() {
        SupportedDescriptors::XOnly(Descriptor::Tr(tree)) => (
            *tree.internal_key(),
            tree.leaves()
                .map(|leaf| leaf.miniscript().encode())
                .collect::<Vec<_>>(),
        ),
        SupportedDescriptors::Taproot(tree) => (
            tree.internal_key(),
            tree.leaves()
                .iter()
                .map(|(_, script)| script.clone())
                .collect(),
        ),
        _ => panic!("expected taproot"),
    };
    assert!(!authorities.contains(&internal));
    assert_eq!(scripts.len(), object.ctv_to_tx.len());
    for script in scripts {
        let decoded = Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(&script).unwrap();
        assert_eq!(decoded.encode(), script);
        assert!(script
            .instructions()
            .any(|instruction| { instruction == Ok(Instruction::Op(OP_NOP4)) }));
    }
    assert!(object.suggested_txs.is_empty());
    assert!(object.program_policies.is_empty());
}

#[test]
fn fixed_vault_has_exact_routes_fees_and_confirmation_delays() {
    let terms = vault();
    let object = terms.call(context(100_000)).unwrap();
    object.validate().unwrap();
    assert_eq!(object, terms.call(context(100_000)).unwrap());
    assert_eq!(object.required_input_amount.to_sat(), 100_000);
    assert_eq!(object.ctv_to_tx.len(), 2);
    let authorities: Vec<_> = terms
        .trigger
        .keys
        .iter()
        .chain(&terms.release.authorization.keys)
        .chain(&terms.recovery.authorization.keys)
        .copied()
        .collect();
    assert_guarded_templates_only(&object, &authorities);

    let trigger = named_template(&object, "pending");
    assert_eq!(trigger.guards, vec![terms.trigger.policy().unwrap()]);
    let direct_recovery = named_template(&object, "recovery");
    assert_eq!(
        direct_recovery.guards,
        vec![terms.recovery.authorization.policy().unwrap()]
    );
    for template in [trigger, direct_recovery] {
        assert_eq!(template.tx.input.len(), 1);
        assert_eq!(template.tx.output.len(), 1);
        assert_eq!(template.tx.output[0].value.to_sat(), 99_500);
        assert_eq!(
            template.funding_constraints.as_ref().unwrap().maximum_fee,
            Amount::from_sat(500)
        );
        assert_eq!(template.tx.input[0].sequence.to_consensus_u32(), 1 << 22);
    }
    let recovery_script = terms
        .recovery
        .destination
        .checked(Network::Regtest)
        .unwrap()
        .script_pubkey();
    assert_eq!(direct_recovery.tx.output[0].script_pubkey, recovery_script);

    let pending = &trigger.outputs[0].contract;
    assert_eq!(pending.ctv_to_tx.len(), 2);
    assert_eq!(pending.required_input_amount.to_sat(), 99_500);
    assert_guarded_templates_only(pending, &authorities);
    let release = named_template(pending, "release");
    let recovery = named_template(pending, "recovery");
    assert_eq!(release.tx.input[0].sequence.to_consensus_u32(), 144);
    assert_eq!(recovery.tx.input[0].sequence.to_consensus_u32(), 1 << 22);
    assert_eq!(
        release.guards,
        vec![terms.release.authorization.policy().unwrap()]
    );
    assert_eq!(
        recovery.guards,
        vec![terms.recovery.authorization.policy().unwrap()]
    );
    for template in [release, recovery] {
        assert_eq!(template.tx.output.len(), 1);
        assert_eq!(template.tx.output[0].value.to_sat(), 99_000);
        assert_eq!(
            template.funding_constraints.as_ref().unwrap().maximum_fee,
            Amount::from_sat(500)
        );
    }
    assert_eq!(
        release.tx.output[0].script_pubkey,
        terms
            .release
            .destination
            .checked(Network::Regtest)
            .unwrap()
            .script_pubkey()
    );
    assert_eq!(recovery.tx.output[0].script_pubkey, recovery_script);
    assert_eq!(
        ScriptBuf::from(direct_recovery.outputs[0].contract.address.clone()),
        recovery_script
    );
}

#[test]
fn fixed_vault_funding_covers_both_fees_and_a_positive_final_output() {
    let terms = vault();
    for funds in [0, 499, 500, 999, 1000] {
        assert!(terms.call(context(funds)).is_err());
    }
    assert!(terms.call(context(1001)).is_ok());
    assert!(terms.call(context(Amount::MAX_MONEY.to_sat() + 1)).is_err());
    let mut huge_fee = terms;
    huge_fee.fee_sats = u64::MAX;
    assert!(huge_fee.call(context(u64::MAX)).is_err());
}

#[test]
fn delayed_wallet_has_immediate_recovery_and_no_covenant_claim() {
    let terms = DelayedWallet {
        hot: signer(1),
        delay: RelativeDelay { blocks: 144 },
        recovery: signer(2),
    };
    let wallet = terms.call(context(100_000)).unwrap();
    wallet.validate().unwrap();
    assert!(wallet.ctv_to_tx.is_empty());
    assert!(wallet.suggested_txs.is_empty());
    assert!(wallet.covenant_requirements.predicates.is_empty());
    let SupportedDescriptors::XOnly(Descriptor::Tr(tree)) = wallet.descriptor.as_ref().unwrap()
    else {
        panic!("expected miniscript taproot");
    };
    assert_eq!(*tree.internal_key(), key(2));
    assert_eq!(tree.leaves().count(), 1);
    let leaf = tree.leaves().next().unwrap();
    let script = leaf.miniscript().to_string();
    assert!(script.contains("older(144)"));
    assert!(script.contains(&key(1).to_string()));
    assert!(!script.contains("ctv("));
    assert!(terms.call(context(0)).is_err());
    assert!(terms.call(context(Amount::MAX_MONEY.to_sat() + 1)).is_err());
    assert!(DelayedWallet {
        delay: RelativeDelay { blocks: 0 },
        ..terms.clone()
    }
    .call(context(100_000))
    .is_err());
    assert!(DelayedWallet {
        recovery: KeySet {
            threshold: 0,
            keys: vec![]
        },
        ..terms
    }
    .call(context(100_000))
    .is_err());
}
