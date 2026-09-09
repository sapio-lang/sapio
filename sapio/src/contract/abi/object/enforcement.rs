// Copyright Judica, Inc 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Enforcement assumptions visible in an artifact's known spending scripts.

use super::{Object, SupportedDescriptors};
use bitcoin::blockdata::opcodes::all::OP_NOP4;
use bitcoin::blockdata::script::Instruction;
use bitcoin::Script;
use sapio_base::miniscript::{Descriptor, MiniscriptKey, ToPublicKey};

fn contains_native_ctv(script: &Script) -> bool {
    script
        .instructions()
        .any(|instruction| matches!(instruction, Ok(Instruction::Op(op)) if op == OP_NOP4))
}

fn descriptor_requires_native_ctv<Pk: MiniscriptKey + ToPublicKey>(
    descriptor: &Descriptor<Pk>,
) -> bool {
    match descriptor {
        Descriptor::Tr(tree) => tree
            .iter_scripts()
            .any(|(_, miniscript)| contains_native_ctv(&miniscript.encode())),
        Descriptor::Wsh(wsh) => contains_native_ctv(&wsh.inner_script()),
        Descriptor::Sh(sh) => contains_native_ctv(&sh.inner_script()),
        Descriptor::Bare(bare) => contains_native_ctv(&bare.inner_script()),
        // These fixed scripts have no CTV instruction; hash/key bytes are data.
        Descriptor::Pkh(_) | Descriptor::Wpkh(_) => false,
    }
}

impl Object {
    /// Whether a known spending script contains the opcode assigned to native
    /// CTV, including outputs of committed and suggested descendant templates.
    ///
    /// Scans all alternative leaves and untaken branches conservatively. Pushed
    /// bytes are data, including inscription bodies, and do not count as CTV.
    /// Address-only destinations have no known spending script to inspect.
    /// This does not establish chain activation or prove a particular spend
    /// executes CTV. Artifact consistency is checked separately by
    /// [`Object::validate`].
    pub fn requires_native_ctv(&self) -> bool {
        let mut pending = vec![self];
        while let Some(object) = pending.pop() {
            let required = match &object.descriptor {
                Some(SupportedDescriptors::Pk(descriptor)) => {
                    descriptor_requires_native_ctv(descriptor)
                }
                Some(SupportedDescriptors::XOnly(descriptor)) => {
                    descriptor_requires_native_ctv(descriptor)
                }
                Some(SupportedDescriptors::Taproot(tree)) => tree
                    .leaves()
                    .iter()
                    .any(|(_, script)| contains_native_ctv(script)),
                None => false,
            };
            if required {
                return true;
            }
            pending.extend(
                object
                    .ctv_to_tx
                    .values()
                    .chain(object.suggested_txs.values())
                    .flat_map(|template| template.outputs.iter())
                    .map(|output| &output.contract),
            );
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::abi::object::RawTaproot;
    use crate::contract::actions::Guard;
    use crate::contract::{Compilable, Context, DynamicContract};
    use crate::template::Template;
    use bitcoin::blockdata::opcodes::all;
    use bitcoin::blockdata::script::Builder;
    use bitcoin::hashes::{sha256, Hash};
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use bitcoin::util::bip32::{ExtendedPrivKey, ExtendedPubKey};
    use bitcoin::{Amount, Network, PublicKey, XOnlyPublicKey};
    use sapio_base::miniscript::{policy::Concrete, Segwitv0};
    use sapio_base::Clause;
    use sapio_base::{Ctv, LoweringPlan};
    use std::sync::Arc;

    fn key() -> XOnlyPublicKey {
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[1; 32]).unwrap())
            .x_only_public_key()
            .0
    }

    fn context() -> Context {
        Context::new(
            Network::Regtest,
            Amount::from_sat(1000),
            LoweringPlan::CtvEmulation {
                signers: vec![ExtendedPubKey::from_priv(
                    &Secp256k1::new(),
                    &ExtendedPrivKey::new_master(Network::Regtest, &[7; 32]).unwrap(),
                )],
                threshold: 1,
            },
            "enforcement".try_into().unwrap(),
            Arc::new(Default::default()),
            None,
        )
    }

    fn raw(script: Script) -> Object {
        raw_leaves(vec![script])
    }

    fn raw_leaves(scripts: Vec<Script>) -> Object {
        let tree = RawTaproot::from_scripts(key(), scripts).unwrap();
        let mut object =
            Object::from_script(tree.script_pubkey(), Amount::ZERO, Network::Regtest).unwrap();
        object.descriptor = Some(tree.into());
        object
    }

    fn ctv_script() -> Script {
        Builder::new()
            .push_slice(&[42; 32])
            .push_opcode(OP_NOP4)
            .push_opcode(all::OP_DROP)
            .push_int(1)
            .into_script()
    }

    #[test]
    fn raw_scripts_distinguish_opcodes_from_data_and_inspect_untaken_branches() {
        let pushed_byte = Builder::new()
            .push_slice(&[OP_NOP4.into_u8()])
            .push_opcode(all::OP_DROP)
            .push_int(1)
            .into_script();
        assert!(!raw(pushed_byte).requires_native_ctv());
        assert!(raw(ctv_script()).requires_native_ctv());
        assert!(
            raw_leaves(vec![Builder::new().push_int(1).into_script(), ctv_script()])
                .requires_native_ctv()
        );

        let untaken = Builder::new()
            .push_int(0)
            .push_opcode(all::OP_IF)
            .push_slice(&[42; 32])
            .push_opcode(OP_NOP4)
            .push_opcode(all::OP_DROP)
            .push_opcode(all::OP_ENDIF)
            .push_int(1)
            .into_script();
        assert!(raw(untaken).requires_native_ctv());
    }

    #[test]
    fn witness_scripts_are_scanned_inside_native_and_nested_wsh_descriptors() {
        let miniscript = Concrete::<PublicKey>::TxTemplate(sha256::Hash::hash(b"template"))
            .compile::<Segwitv0>()
            .unwrap();
        for descriptor in [
            Descriptor::new_wsh(miniscript.clone()).unwrap(),
            Descriptor::new_sh_wsh(miniscript).unwrap(),
        ] {
            let object = Object::from_descriptor(descriptor, Amount::ZERO);
            assert!(object.requires_native_ctv());
        }
    }

    fn finish_guard() -> Option<Guard<Clause>> {
        Some(Guard::Cache(Clone::clone, None))
    }

    fn finish(clause: Clause) -> Object {
        DynamicContract::<(), _> {
            then: vec![],
            finish_or: vec![],
            finish: vec![finish_guard],
            metadata_f: Box::new(|_, _| Ok(Default::default())),
            ensure_amount_f: Box::new(|_, _| Ok(Amount::ZERO)),
            data: clause,
        }
        .compile(context())
        .unwrap()
    }

    #[test]
    fn finish_only_ctv_is_detected_despite_a_signer_emulator_context() {
        let object = finish(Clause::TxTemplate(sha256::Hash::hash(b"template")));
        object.validate().unwrap();
        assert!(object.ctv_to_tx.is_empty());
        assert!(object.suggested_txs.is_empty());
        assert!(object.requires_native_ctv());
        assert!(!finish(Clause::Key(key())).requires_native_ctv());
        // A signature-only alternative does not hide a CTV-bearing leaf.
        assert!(finish(Clause::Threshold(
            1,
            vec![
                Clause::Key(key()),
                Clause::TxTemplate(sha256::Hash::hash(b"template")),
            ],
        ))
        .requires_native_ctv());
    }

    fn parent(child: &Object, suggested: bool) -> Object {
        let template: Template = context()
            .template()
            .add_output(Amount::from_sat(1000), child, None)
            .unwrap()
            .into();
        let mut object = finish(Clause::Key(key()));
        object.required_input_amount = template.required_input_amount;
        if !suggested {
            object
                .covenant_requirements
                .predicates
                .insert(Ctv(template.hash()));
        }
        let templates = if suggested {
            &mut object.suggested_txs
        } else {
            &mut object.ctv_to_tx
        };
        templates.insert(template.hash(), template);
        object.validate().unwrap();
        object
    }

    #[test]
    fn detection_visits_committed_and_suggested_descendants_at_every_level() {
        for outer_suggested in [false, true] {
            for inner_suggested in [false, true] {
                let leaf = raw(ctv_script());
                let descendant = parent(&leaf, inner_suggested);
                assert!(parent(&descendant, outer_suggested).requires_native_ctv());
                let ordinary = finish(Clause::Key(key()));
                let descendant = parent(&ordinary, inner_suggested);
                assert!(!parent(&descendant, outer_suggested).requires_native_ctv());
            }
        }
    }

    #[test]
    fn address_only_destination_does_not_claim_to_know_the_committed_script() {
        let known = raw(ctv_script());
        let unknown = Object::from_script(
            known.descriptor.as_ref().unwrap().script_pubkey(),
            Amount::ZERO,
            Network::Regtest,
        )
        .unwrap();
        assert!(known.requires_native_ctv());
        assert!(!unknown.requires_native_ctv());
    }
}
