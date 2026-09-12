// Copyright Judica, Inc 2022
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Wrapper for supported descriptor types

use super::RawTaproot;
pub use crate::contract::abi::studio::*;
use bitcoin::psbt::Input;
use bitcoin::taproot::ControlBlock;
use bitcoin::PublicKey;
use bitcoin::ScriptBuf;
use bitcoin::XOnlyPublicKey;
use miniscript::*;
use sapio_base::miniscript;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Multiple Types of Allowed Descriptor
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum SupportedDescriptors {
    /// # ECDSA Descriptors
    Pk(Descriptor<PublicKey>),
    /// # Taproot Descriptors
    XOnly(Descriptor<XOnlyPublicKey>),
    /// # Checked raw Taproot scripts
    /// Spending data for scripts whose witness requirements are not described
    /// by Miniscript. This representation carries no satisfaction-weight bound.
    Taproot(RawTaproot),
}

impl From<Descriptor<PublicKey>> for SupportedDescriptors {
    fn from(x: Descriptor<PublicKey>) -> Self {
        SupportedDescriptors::Pk(x)
    }
}
impl From<Descriptor<XOnlyPublicKey>> for SupportedDescriptors {
    fn from(x: Descriptor<XOnlyPublicKey>) -> Self {
        SupportedDescriptors::XOnly(x)
    }
}
impl From<RawTaproot> for SupportedDescriptors {
    fn from(tree: RawTaproot) -> Self {
        SupportedDescriptors::Taproot(tree)
    }
}
impl SupportedDescriptors {
    /// Add the descriptor's spending scripts and Taproot proofs to an input.
    ///
    /// Existing funding, signatures and other PSBT fields are retained. Raw
    /// Taproot trees retain every control block when a script appears at
    /// multiple positions in the tree. The caller remains responsible for
    /// matching this descriptor to the spent output.
    pub fn update_psbt_input(&self, input: &mut Input) -> Result<(), miniscript::Error> {
        match self {
            Self::Pk(descriptor) => input.witness_script = Some(descriptor.explicit_script()?),
            Self::XOnly(Descriptor::Tr(tree)) => {
                let info = tree.spend_info();
                for leaf in info.leaves() {
                    let script = (leaf.script().to_owned(), leaf.leaf_version());
                    input.tap_scripts.insert(leaf.into_control_block(), script);
                }
                input.tap_merkle_root = info.merkle_root();
                input.tap_internal_key = Some(info.internal_key());
            }
            Self::Taproot(tree) => {
                let info = tree.spend_info();
                for ((script, version), branches) in info.script_map() {
                    for branch in branches {
                        let control = ControlBlock {
                            leaf_version: *version,
                            output_key_parity: info.output_key_parity(),
                            internal_key: info.internal_key(),
                            merkle_branch: branch.clone(),
                        };
                        input
                            .tap_scripts
                            .insert(control, (script.clone(), *version));
                    }
                }
                input.tap_merkle_root = info.merkle_root();
                input.tap_internal_key = Some(info.internal_key());
            }
            Self::XOnly(_) => (),
        }
        Ok(())
    }

    /// Regardless of descriptor type, get the output script
    pub fn script_pubkey(&self) -> ScriptBuf {
        match self {
            SupportedDescriptors::Pk(p) => p.script_pubkey(),
            SupportedDescriptors::XOnly(x) => x.script_pubkey(),
            SupportedDescriptors::Taproot(tree) => tree.script_pubkey(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::psbt::raw::Key;
    use bitcoin::secp256k1::{Keypair, Message, Secp256k1, SecretKey};
    use bitcoin::taproot::LeafVersion;
    use bitcoin::{Amount, TapSighashType, TxOut};
    use std::str::FromStr;

    fn keypair(seed: u8) -> Keypair {
        Keypair::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[seed; 32]).unwrap(),
        )
    }

    fn existing_input() -> Input {
        Input {
            witness_utxo: Some(TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }),
            tap_key_sig: Some(bitcoin::taproot::Signature {
                signature: Secp256k1::new()
                    .sign_schnorr_no_aux_rand(&Message::from_digest([1; 32]), &keypair(1)),
                sighash_type: TapSighashType::All,
            }),
            unknown: [(
                Key {
                    type_value: 0xff,
                    key: vec![1],
                },
                vec![2],
            )]
            .into(),
            ..Input::default()
        }
    }

    #[test]
    fn raw_taproot_retains_all_duplicate_leaf_proofs_and_existing_input_data() {
        let key = keypair(1).x_only_public_key().0;
        let repeated = ScriptBuf::from(vec![0x51]);
        let other = ScriptBuf::from(vec![0x51, 0x6b, 0x6c]);
        let tree = RawTaproot::new(
            key,
            vec![(1, repeated.clone()), (2, repeated.clone()), (2, other)],
        )
        .unwrap();
        let descriptor = SupportedDescriptors::Taproot(tree.clone());
        let original = existing_input();
        let mut input = original.clone();
        descriptor.update_psbt_input(&mut input).unwrap();

        assert_eq!(input.tap_internal_key, Some(key));
        assert_eq!(input.tap_merkle_root, tree.spend_info().merkle_root());
        assert_eq!(input.tap_scripts.len(), 3);
        assert_eq!(
            input
                .tap_scripts
                .values()
                .filter(|(script, _)| script == &repeated)
                .count(),
            2
        );
        for (control, (script, version)) in &input.tap_scripts {
            assert_eq!(*version, LeafVersion::TapScript);
            assert!(control.verify_taproot_commitment(
                &Secp256k1::verification_only(),
                tree.spend_info().output_key().to_x_only_public_key(),
                script,
            ));
        }
        input.tap_internal_key = None;
        input.tap_merkle_root = None;
        input.tap_scripts.clear();
        assert_eq!(input, original);
    }

    #[test]
    fn miniscript_taproot_supplies_verifiable_proofs_and_key_only_outputs() {
        let key = keypair(1).x_only_public_key().0;
        let leaf_key = keypair(2).x_only_public_key().0;
        for (expression, leaves) in [
            (format!("tr({key},pk({leaf_key}))"), 1),
            (format!("tr({key})"), 0),
        ] {
            let descriptor = Descriptor::<XOnlyPublicKey>::from_str(&expression).unwrap();
            let Descriptor::Tr(tree) = &descriptor else {
                unreachable!()
            };
            let output_key = tree.spend_info().output_key().to_x_only_public_key();
            let mut input = Input::default();
            SupportedDescriptors::XOnly(descriptor)
                .update_psbt_input(&mut input)
                .unwrap();
            assert_eq!(input.tap_internal_key, Some(key));
            assert_eq!(input.tap_scripts.len(), leaves);
            assert_eq!(input.tap_merkle_root.is_some(), leaves != 0);
            for (control, (script, _)) in input.tap_scripts {
                assert!(control.verify_taproot_commitment(
                    &Secp256k1::verification_only(),
                    output_key,
                    &script,
                ));
            }
        }
    }

    #[test]
    fn ecdsa_descriptor_retains_existing_input_data() {
        let key = bitcoin::PublicKey::new(keypair(1).public_key());
        let descriptor = Descriptor::<PublicKey>::from_str(&format!("wsh(pk({key}))")).unwrap();
        let expected = descriptor.explicit_script().unwrap();
        let original = existing_input();
        let mut input = original.clone();
        SupportedDescriptors::Pk(descriptor)
            .update_psbt_input(&mut input)
            .unwrap();
        assert_eq!(input.witness_script.take(), Some(expected));
        assert_eq!(input, original);
    }
}
