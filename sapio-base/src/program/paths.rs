use super::EmulatedProgram;
use super::{ProgramError, ProgramSpendPath};
use crate::miniscript::{Miniscript, Tap};
use bitcoin::psbt::Input;
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::XOnlyPublicKey;

impl ProgramSpendPath {
    /// Select the unique Miniscript leaf containing this program's signing key.
    ///
    /// Other guards may share the leaf. Repeated control blocks for the same
    /// leaf are harmless; distinct matching leaves require explicit selection.
    /// Raw scripts and key-path spends must be selected explicitly. This only
    /// locates a signature slot: the oracle still authenticates the supplied
    /// control block and previous output before evaluating or signing.
    pub fn script_for(program: &EmulatedProgram, input: &Input) -> Result<Self, ProgramError> {
        let key = program.derive_public_key()?;
        let mut selected = None;
        for (script, version) in input.tap_scripts.values() {
            if *version != LeafVersion::TapScript {
                continue;
            }
            let Ok(policy) = Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(script) else {
                continue;
            };
            if !policy.iter_pk().any(|candidate| candidate == key) {
                continue;
            }
            let hash = TapLeafHash::from_script(script, *version);
            if selected.is_some_and(|previous| previous != hash) {
                return Err(ProgramError::AmbiguousProgramLeaf);
            }
            selected = Some(hash);
        }
        selected
            .map(Self::ScriptPath)
            .ok_or(ProgramError::MissingProgramLeaf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragments::{template_signed_by, TemplateKey};
    use bitcoin::bip32::{Xpriv, Xpub};
    use bitcoin::blockdata::opcodes::all::OP_DROP;
    use bitcoin::blockdata::script::Builder;
    use bitcoin::secp256k1::Secp256k1;
    use bitcoin::taproot::{ControlBlock, TaprootBuilder};
    use bitcoin::{Amount, Network, ScriptBuf, TxOut};

    fn root(seed: u8) -> Xpub {
        Xpub::from_priv(
            &Secp256k1::new(),
            &Xpriv::new_master(Network::Testnet, &[seed; 32]).unwrap(),
        )
    }

    fn program() -> EmulatedProgram {
        template_signed_by(TemplateKey::InternalKey, root(107)).unwrap()
    }

    fn script(policy: String) -> ScriptBuf {
        policy
            .parse::<Miniscript<XOnlyPublicKey, Tap>>()
            .unwrap()
            .encode()
    }

    fn input(leaves: Vec<(u8, ScriptBuf, LeafVersion)>) -> Input {
        let secp = Secp256k1::verification_only();
        let mut builder = TaprootBuilder::new();
        for (depth, script, version) in leaves {
            builder = builder.add_leaf_with_ver(depth, script, version).unwrap();
        }
        let info = builder
            .finalize(&secp, root(108).public_key.x_only_public_key().0)
            .unwrap();
        let mut input = Input {
            witness_utxo: Some(TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new_p2tr_tweaked(info.output_key()),
            }),
            tap_internal_key: Some(info.internal_key()),
            tap_merkle_root: info.merkle_root(),
            ..Input::default()
        };
        for ((script, version), branches) in info.script_map() {
            for branch in branches {
                let control = ControlBlock {
                    leaf_version: *version,
                    output_key_parity: info.output_key_parity(),
                    internal_key: info.internal_key(),
                    merkle_branch: branch.clone(),
                };
                assert!(control.verify_taproot_commitment(
                    &secp,
                    info.output_key().to_x_only_public_key(),
                    script,
                ));
                input
                    .tap_scripts
                    .insert(control, (script.clone(), *version));
            }
        }
        input
    }

    #[test]
    fn selects_program_key_within_a_composite_timelock_guard() {
        let program = program();
        let key = program.derive_public_key().unwrap();
        let other = root(109).public_key.x_only_public_key().0;
        let guarded = script(format!("and_v(v:older(6),pk({key}))"));
        let input = input(vec![
            (1, guarded.clone(), LeafVersion::TapScript),
            (1, script(format!("pk({other})")), LeafVersion::TapScript),
        ]);
        assert_eq!(
            ProgramSpendPath::script_for(&program, &input).unwrap(),
            ProgramSpendPath::ScriptPath(TapLeafHash::from_script(
                &guarded,
                LeafVersion::TapScript
            ))
        );
    }

    #[test]
    fn repeated_control_proofs_for_one_leaf_do_not_make_selection_ambiguous() {
        let program = program();
        let key = program.derive_public_key().unwrap();
        let leaf = script(format!("pk({key})"));
        let other = root(109).public_key.x_only_public_key().0;
        let input = input(vec![
            (1, leaf.clone(), LeafVersion::TapScript),
            (2, leaf.clone(), LeafVersion::TapScript),
            (2, script(format!("pk({other})")), LeafVersion::TapScript),
        ]);
        assert_eq!(input.tap_scripts.len(), 3);
        assert_eq!(
            input
                .tap_scripts
                .values()
                .filter(|(script, _)| *script == leaf)
                .count(),
            2
        );
        assert_eq!(
            ProgramSpendPath::script_for(&program, &input).unwrap(),
            ProgramSpendPath::ScriptPath(TapLeafHash::from_script(&leaf, LeafVersion::TapScript))
        );
    }

    #[test]
    fn distinct_leaves_requiring_the_program_key_need_explicit_selection() {
        let program = program();
        let key = program.derive_public_key().unwrap();
        let input = input(vec![
            (1, script(format!("pk({key})")), LeafVersion::TapScript),
            (
                1,
                script(format!("and_v(v:older(6),pk({key}))")),
                LeafVersion::TapScript,
            ),
        ]);
        assert!(matches!(
            ProgramSpendPath::script_for(&program, &input),
            Err(ProgramError::AmbiguousProgramLeaf)
        ));
    }

    #[test]
    fn program_key_bytes_used_as_hash_data_are_not_a_signature_slot() {
        let program = program();
        let key = program.derive_public_key().unwrap();
        let other = root(109).public_key.x_only_public_key().0;
        let leaf = script(format!("and_v(v:sha256({key}),pk({other}))"));
        assert!(leaf
            .as_bytes()
            .windows(32)
            .any(|bytes| bytes == key.serialize()));
        let input = input(vec![(0, leaf, LeafVersion::TapScript)]);
        assert!(matches!(
            ProgramSpendPath::script_for(&program, &input),
            Err(ProgramError::MissingProgramLeaf)
        ));
    }

    #[test]
    fn missing_keys_raw_scripts_and_future_leaf_versions_are_not_selected() {
        let program = program();
        let key = program.derive_public_key().unwrap();
        let other = root(109).public_key.x_only_public_key().0;
        let raw = Builder::new()
            .push_slice(key.serialize())
            .push_opcode(OP_DROP)
            .push_int(1)
            .into_script();
        for candidate in [
            Input::default(),
            input(vec![(
                0,
                script(format!("pk({other})")),
                LeafVersion::TapScript,
            )]),
            input(vec![(0, raw, LeafVersion::TapScript)]),
            input(vec![(
                0,
                script(format!("pk({key})")),
                LeafVersion::from_consensus(0xc2).unwrap(),
            )]),
        ] {
            assert!(matches!(
                ProgramSpendPath::script_for(&program, &candidate),
                Err(ProgramError::MissingProgramLeaf)
            ));
        }
    }
}
