// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Necessary transaction-field checks for the Miniscript policy backend.

use bitcoin::hashes::sha256;
use bitcoin::Transaction;
use sapio_base::{CTVHash, Clause};

const LOCK_TIME_THRESHOLD: u32 = 500_000_000;
const SEQUENCE_DISABLE_FLAG: u32 = 1 << 31;
const SEQUENCE_TYPE_FLAG: u32 = 1 << 22;
const SEQUENCE_LOCK_MASK: u32 = SEQUENCE_TYPE_FLAG | 0xffff;

/// Whether a Miniscript policy has an authorization compatible with the fixed
/// transaction fields at the spending input. Keys and preimages are assumed
/// available; this does not establish chain maturity, funding, or spendability.
/// The CTV commitment is recomputed rather than taken from template metadata.
pub(crate) fn miniscript_policy_possible(
    policy: &Clause,
    tx: &Transaction,
    input_index: u32,
) -> bool {
    let Some(input) = tx.input.get(input_index as usize) else {
        return false;
    };
    policy_possible(policy, tx, input.sequence, tx.get_ctv_hash(input_index))
}

fn policy_possible(policy: &Clause, tx: &Transaction, sequence: u32, ctv: sha256::Hash) -> bool {
    let possible = |policy| policy_possible(policy, tx, sequence, ctv);
    match policy {
        Clause::Unsatisfiable => false,
        Clause::Trivial
        | Clause::Key(_)
        | Clause::Sha256(_)
        | Clause::Hash256(_)
        | Clause::Ripemd160(_)
        | Clause::Hash160(_) => true,
        Clause::After(required) => {
            sequence != u32::MAX
                && (tx.lock_time < LOCK_TIME_THRESHOLD) == (*required < LOCK_TIME_THRESHOLD)
                && *required <= tx.lock_time
        }
        Clause::Older(required) => {
            // BIP 112 treats an operand with its disable bit set as a NOP,
            // before inspecting transaction version or input sequence.
            required & SEQUENCE_DISABLE_FLAG != 0
                || (tx.version >= 2
                    && sequence & SEQUENCE_DISABLE_FLAG == 0
                    && (required & SEQUENCE_TYPE_FLAG) == (sequence & SEQUENCE_TYPE_FLAG)
                    && (required & SEQUENCE_LOCK_MASK) <= (sequence & SEQUENCE_LOCK_MASK))
        }
        Clause::TxTemplate(required) => *required == ctv,
        Clause::And(children) => children.iter().all(possible),
        Clause::Or(children) => children.iter().any(|(_, child)| possible(child)),
        Clause::Threshold(required, children) => {
            children.iter().filter(|child| possible(child)).count() >= *required
        }
        Clause::Inscribe(_, child) => possible(child),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::{hash160, ripemd160, sha256d, Hash};
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use bitcoin::{TxIn, TxOut, Witness};
    use sapio_base::miniscript::ord::Inscription;

    fn transaction(version: i32, lock_time: u32, sequence: u32) -> Transaction {
        Transaction {
            version,
            lock_time,
            input: vec![TxIn {
                previous_output: Default::default(),
                script_sig: Default::default(),
                sequence,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: 1_000,
                script_pubkey: Default::default(),
            }],
        }
    }

    fn key_policy() -> Clause {
        let key =
            Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[1; 32]).unwrap())
                .x_only_public_key()
                .0;
        Clause::Key(key)
    }

    #[test]
    fn credentials_are_potentially_available() {
        let tx = transaction(2, 0, 0);
        for policy in [
            Clause::Trivial,
            key_policy(),
            Clause::Sha256(sha256::Hash::hash(b"unknown preimage")),
            Clause::Hash256(sha256d::Hash::hash(b"unknown preimage")),
            Clause::Ripemd160(ripemd160::Hash::hash(b"unknown preimage")),
            Clause::Hash160(hash160::Hash::hash(b"unknown preimage")),
        ] {
            assert!(miniscript_policy_possible(&policy, &tx, 0));
        }
        assert!(!miniscript_policy_possible(&Clause::Unsatisfiable, &tx, 0));
    }

    #[test]
    fn absolute_locktime_requires_same_domain_and_nonfinal_input() {
        for (required, lock_time, sequence, expected) in [
            (100, 99, 0, false),
            (100, 100, 0, true),
            (100, 101, 0, true),
            (100, 100, u32::MAX, false),
            (100, 100, u32::MAX - 1, true),
            (LOCK_TIME_THRESHOLD - 1, LOCK_TIME_THRESHOLD - 1, 0, true),
            (LOCK_TIME_THRESHOLD - 1, LOCK_TIME_THRESHOLD, 0, false),
            (LOCK_TIME_THRESHOLD, LOCK_TIME_THRESHOLD - 1, 0, false),
            (LOCK_TIME_THRESHOLD, LOCK_TIME_THRESHOLD, 0, true),
            (LOCK_TIME_THRESHOLD + 1, LOCK_TIME_THRESHOLD, 0, false),
            (u32::MAX, u32::MAX, 0, true),
        ] {
            assert_eq!(
                miniscript_policy_possible(
                    &Clause::After(required),
                    &transaction(1, lock_time, sequence),
                    0,
                ),
                expected,
                "after({required}), lock_time={lock_time}, sequence={sequence}"
            );
        }
    }

    #[test]
    fn relative_locktime_requires_enabled_version_and_sequence() {
        for version in [i32::MIN, -1, 0, 1, 2, 3, i32::MAX] {
            assert_eq!(
                miniscript_policy_possible(&Clause::Older(1), &transaction(version, 0, 1), 0),
                version >= 2,
                "version={version}"
            );
            for sequence in [SEQUENCE_DISABLE_FLAG | 1, u32::MAX] {
                assert!(!miniscript_policy_possible(
                    &Clause::Older(1),
                    &transaction(version, 0, sequence),
                    0,
                ));
            }
        }
    }

    #[test]
    fn disabled_csv_operand_is_a_nop() {
        for required in [SEQUENCE_DISABLE_FLAG, u32::MAX] {
            for version in [-1, 1, 2] {
                for sequence in [0, SEQUENCE_TYPE_FLAG, SEQUENCE_DISABLE_FLAG, u32::MAX] {
                    assert!(miniscript_policy_possible(
                        &Clause::Older(required),
                        &transaction(version, 0, sequence),
                        0,
                    ));
                }
            }
        }
    }

    #[test]
    fn relative_locktime_compares_only_consensus_bits_in_the_same_domain() {
        const RESERVED: u32 = 0x7fff_ffff & !SEQUENCE_LOCK_MASK;
        for (required, sequence, expected) in [
            (10, 9, false),
            (10, 10, true),
            (10, 11, true),
            (0xffff, 0xffff, true),
            (10, SEQUENCE_TYPE_FLAG | 10, false),
            (SEQUENCE_TYPE_FLAG | 10, 10, false),
            (SEQUENCE_TYPE_FLAG | 10, SEQUENCE_TYPE_FLAG | 9, false),
            (SEQUENCE_TYPE_FLAG | 10, SEQUENCE_TYPE_FLAG | 10, true),
            (SEQUENCE_TYPE_FLAG | 10, SEQUENCE_TYPE_FLAG | 11, true),
            (RESERVED | 10, 10, true),
            (10, RESERVED | 10, true),
            (RESERVED | 10, RESERVED | 9, false),
            (
                RESERVED | SEQUENCE_TYPE_FLAG | 10,
                SEQUENCE_TYPE_FLAG | 10,
                true,
            ),
        ] {
            assert_eq!(
                miniscript_policy_possible(
                    &Clause::Older(required),
                    &transaction(2, 0, sequence),
                    0,
                ),
                expected,
                "older({required}), sequence={sequence}"
            );
        }
    }

    #[test]
    fn template_commitment_and_timelocks_use_the_selected_input() {
        let mut tx = transaction(2, 100, u32::MAX);
        let mut second_input = tx.input[0].clone();
        second_input.sequence = 10;
        tx.input.push(second_input);
        let committed_to_zero = Clause::TxTemplate(tx.get_ctv_hash(0));
        let committed_to_one = Clause::TxTemplate(tx.get_ctv_hash(1));
        assert!(miniscript_policy_possible(&committed_to_zero, &tx, 0));
        assert!(!miniscript_policy_possible(&committed_to_zero, &tx, 1));
        assert!(miniscript_policy_possible(&committed_to_one, &tx, 1));
        for policy in [Clause::After(100), Clause::Older(10)] {
            assert!(!miniscript_policy_possible(&policy, &tx, 0));
            assert!(miniscript_policy_possible(&policy, &tx, 1));
        }
        tx.output[0].value += 1;
        assert!(!miniscript_policy_possible(&committed_to_zero, &tx, 0));
        assert!(!miniscript_policy_possible(&committed_to_one, &tx, 1));
    }

    #[test]
    fn a_spending_input_must_exist() {
        let mut tx = transaction(2, 100, 10);
        assert!(!miniscript_policy_possible(&Clause::Trivial, &tx, 1));
        assert!(!miniscript_policy_possible(&Clause::Trivial, &tx, u32::MAX));
        tx.input.clear();
        assert!(!miniscript_policy_possible(&Clause::Trivial, &tx, 0));
    }

    #[test]
    fn alternatives_thresholds_and_inscriptions_preserve_possible_authorizations() {
        let tx = transaction(2, 100, 10);
        let impossible = Clause::Older(11);
        let possible = key_policy();
        let alternatives = Clause::Or(vec![(1, impossible.clone()), (0, possible.clone())]);
        assert!(miniscript_policy_possible(&alternatives, &tx, 0));
        assert!(!miniscript_policy_possible(
            &Clause::And(vec![impossible.clone(), possible.clone()]),
            &tx,
            0,
        ));
        let children = vec![impossible.clone(), possible, Clause::After(100)];
        assert!(miniscript_policy_possible(
            &Clause::Threshold(2, children.clone()),
            &tx,
            0,
        ));
        assert!(!miniscript_policy_possible(
            &Clause::Threshold(3, children),
            &tx,
            0,
        ));
        for (inner, expected) in [(alternatives, true), (impossible, false)] {
            let inscription = Clause::Inscribe(
                Box::new(Inscription::new(
                    Some(b"application/octet-stream".to_vec()),
                    Some(vec![0, 1, 255]),
                )),
                Box::new(inner),
            );
            assert_eq!(miniscript_policy_possible(&inscription, &tx, 0), expected);
        }
    }
}
