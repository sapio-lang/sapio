//! Prepare one explicitly selected program requirement from a compiled artifact.

use super::{
    prepare, previous_outputs, target_signature, ProgramError, ProgramSigningRequest, PSBT,
};
use bitcoin::psbt::{Input, Psbt};
use bitcoin::{Amount, ScriptBuf};
use sapio::contract::abi::object::{ArtifactError, Object, ProgramRequirement};
use sapio_base::miniscript;
use std::fmt;

/// A compiled artifact and funded PSBT cannot prepare the selected program spend.
#[derive(Debug)]
pub enum ArtifactProgramError {
    /// The compiled artifact contains inconsistent spending data.
    Artifact(ArtifactError),
    /// The selected program and path are not a requirement of this artifact.
    UnknownRequirement,
    /// The artifact supplies no descriptor from which to obtain spending proofs.
    MissingDescriptor,
    /// The selected input spends a different script from the artifact's output.
    PrevoutMismatch,
    /// The selected input cannot fund the contract's declared requirement.
    UnderfundedInput {
        /// Value supplied by the selected previous output.
        available: Amount,
        /// Minimum value declared by the compiled contract.
        required: Amount,
    },
    /// The descriptor could not provide its spending data.
    Descriptor(miniscript::Error),
    /// The PSBT or selected signature request violates the program protocol.
    Program(ProgramError),
}

impl fmt::Display for ArtifactProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Artifact(error) => error.fmt(formatter),
            Self::UnknownRequirement => {
                formatter.write_str("selected program and path are absent from the artifact")
            }
            Self::MissingDescriptor => formatter.write_str("artifact has no spending descriptor"),
            Self::PrevoutMismatch => {
                formatter.write_str("selected input does not spend the artifact's output script")
            }
            Self::UnderfundedInput {
                available,
                required,
            } => write!(
                formatter,
                "selected input provides {} sat; the artifact requires {} sat",
                available.to_sat(),
                required.to_sat(),
            ),
            Self::Descriptor(error) => error.fmt(formatter),
            Self::Program(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ArtifactProgramError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Artifact(error) => Some(error),
            Self::Descriptor(error) => Some(error),
            Self::Program(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProgramError> for ArtifactProgramError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}

/// Validate and prepare one program request without evaluating or signing it.
///
/// The caller selects a requirement, input and auxiliary witness explicitly.
/// Every previous output must be available, and the selected output must match
/// the artifact's script and funding requirement. Missing descriptor proofs are
/// supplied; conflicting proofs are rejected. All other PSBT fields survive.
/// Optional contract metadata does not select or authorize a program.
///
/// Submit the result to an explicitly configured [`super::ProgramOracle`] or
/// [`super::ProgramClient`]. Preparation does not contact either one or check
/// the evaluator's predicate, native timelocks or other script requirements.
pub fn prepare_program_request(
    object: &Object,
    requirement: &ProgramRequirement,
    mut psbt: Psbt,
    input_index: u32,
    witness: Vec<u8>,
) -> Result<ProgramSigningRequest, ArtifactProgramError> {
    let requirements = object
        .program_requirements()
        .map_err(ArtifactProgramError::Artifact)?;
    if !requirements.contains(requirement) {
        return Err(ArtifactProgramError::UnknownRequirement);
    }
    sapio_psbt::validate_psbt(&psbt).map_err(ProgramError::Psbt)?;
    let prevouts = previous_outputs(&psbt)?;
    let prevout = prevouts
        .get(input_index as usize)
        .ok_or(ProgramError::InputIndex(input_index))?;
    if prevout.script_pubkey != ScriptBuf::from(&object.address) {
        return Err(ArtifactProgramError::PrevoutMismatch);
    }
    if prevout.value < object.required_input_amount {
        return Err(ArtifactProgramError::UnderfundedInput {
            available: prevout.value,
            required: object.required_input_amount,
        });
    }

    let mut expected = Input::default();
    object
        .descriptor
        .as_ref()
        .ok_or(ArtifactProgramError::MissingDescriptor)?
        .update_psbt_input(&mut expected)
        .map_err(ArtifactProgramError::Descriptor)?;
    let input = &mut psbt.inputs[input_index as usize];
    if input
        .tap_internal_key
        .is_some_and(|key| Some(key) != expected.tap_internal_key)
        || input
            .tap_merkle_root
            .is_some_and(|root| Some(root) != expected.tap_merkle_root)
        || input
            .tap_scripts
            .iter()
            .any(|(control, leaf)| expected.tap_scripts.get(control) != Some(leaf))
    {
        return Err(ProgramError::ConflictingTaprootMetadata.into());
    }
    input.tap_internal_key = expected.tap_internal_key;
    input.tap_merkle_root = expected.tap_merkle_root;
    input.tap_scripts.extend(expected.tap_scripts);

    let request = ProgramSigningRequest {
        instance: requirement.program.instance().clone(),
        input_index,
        witness,
        path: requirement.path,
        psbt: PSBT(psbt),
    };
    let prepared = prepare(&request, requirement.program.root())?;
    if target_signature(&request.psbt.0, &request, prepared.program_key)
        .is_some_and(|signature| !prepared.verify(signature))
    {
        return Err(ProgramError::ConflictingSignature.into());
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{validate_program_response, ProgramOracle, ProgramSpendPath};
    use bitcoin::bip32::{Xpriv, Xpub};
    use bitcoin::hashes::Hash;
    use bitcoin::psbt::raw;
    use bitcoin::secp256k1::Secp256k1;
    use bitcoin::taproot::{TapLeafHash, TapNodeHash};
    use bitcoin::{Network, OutPoint, Sequence, Transaction, TxIn, TxOut};
    use sapio::contract::{Compilable, CompilationError, Context, Contract};
    use sapio::{declare, guard};
    use sapio_base::covenant::LoweringPlan;
    use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
    use sapio_base::program::{EmulatedProgram, ProgramInstance};
    use sapio_base::Clause;
    use std::sync::Arc;

    fn root() -> Xpriv {
        Xpriv::new_master(Network::Regtest, &[51; 32]).unwrap()
    }

    fn program() -> EmulatedProgram {
        let bytes = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (global $heap (mut i32) (i32.const 4096))
                (func (export "sapio_alloc_v2") (param $length i32) (result i32)
                    global.get $heap
                    global.get $heap local.get $length i32.add global.set $heap)
                (func (export "sapio_evaluate_v2")
                    (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)
                    i32.const 1))"#,
        )
        .unwrap();
        EmulatedProgram::new(
            ProgramInstance::wasm_v2(bytes, vec![]).unwrap(),
            Xpub::from_priv(&Secp256k1::new(), &root()),
        )
        .unwrap()
    }

    struct Predicate {
        program: EmulatedProgram,
        delayed: bool,
    }

    impl Predicate {
        #[guard(policy, cached)]
        fn authorized(self) -> ScriptPolicy {
            let program = self.program.compile_policy().unwrap();
            if self.delayed {
                ScriptPolicy::And(vec![
                    Clause::Older(miniscript::RelLockTime::from_consensus(16).unwrap()).into(),
                    program,
                ])
            } else {
                program
            }
        }
    }

    impl Contract for Predicate {
        declare! {finish, Self::authorized}

        fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
            Ok(Amount::from_sat(10_000))
        }
    }

    fn artifact(delayed: bool) -> Object {
        let bytes = {
            let contract = Predicate {
                program: program(),
                delayed,
            };
            let object = contract
                .compile(Context::new(
                    Network::Regtest,
                    Amount::from_sat(10_000),
                    LoweringPlan::Native,
                    "program_artifact".try_into().unwrap(),
                    Arc::new(Default::default()),
                    None,
                ))
                .unwrap();
            serde_json::to_vec(&object).unwrap()
        };
        serde_json::from_slice(&bytes).unwrap()
    }

    fn requirement(object: &Object, delayed: bool) -> ProgramRequirement {
        object
            .program_requirements()
            .unwrap()
            .into_iter()
            .find(|requirement| {
                matches!(requirement.path, ProgramSpendPath::ScriptPath(_)) == delayed
            })
            .unwrap()
    }

    fn funded(object: &Object) -> Psbt {
        let mut psbt = Psbt::from_unsigned_tx(Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: [1, 2]
                .into_iter()
                .map(|byte| TxIn {
                    previous_output: OutPoint {
                        txid: bitcoin::Txid::from_byte_array([byte; 32]),
                        vout: 0,
                    },
                    sequence: Sequence(16),
                    ..TxIn::default()
                })
                .collect(),
            output: vec![TxOut {
                value: Amount::from_sat(9_000),
                script_pubkey: ScriptBuf::new(),
            }],
        })
        .unwrap();
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new(),
        });
        psbt.inputs[1].witness_utxo = Some(TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: (&object.address).into(),
        });
        let annotation = raw::Key {
            type_value: 0xff,
            key: vec![1],
        };
        psbt.unknown.insert(annotation.clone(), vec![11]);
        psbt.inputs[0].unknown.insert(annotation.clone(), vec![12]);
        psbt.inputs[1].unknown.insert(annotation.clone(), vec![13]);
        psbt.outputs[0].unknown.insert(annotation, vec![14]);
        sapio_psbt::annex::set(&mut psbt.inputs[1], Some(vec![0x50, 0x15])).unwrap();
        psbt
    }

    #[test]
    fn serialized_artifacts_prepare_and_sign_without_contract_metadata() {
        for delayed in [false, true] {
            let mut object = artifact(delayed);
            let requirement = requirement(&object, delayed);
            let original = funded(&object);
            let prepared =
                prepare_program_request(&object, &requirement, original.clone(), 1, vec![21])
                    .unwrap();
            object.metadata = Default::default();
            object.metadata.simp.insert(
                -17,
                serde_json::json!({"program": "untrusted", "endpoint": "untrusted"}),
            );
            assert_eq!(
                prepare_program_request(&object, &requirement, original.clone(), 1, vec![21])
                    .unwrap(),
                prepared
            );
            assert_eq!(prepared.instance, *requirement.program.instance());
            assert_eq!(prepared.path, requirement.path);
            assert_eq!(prepared.witness, vec![21]);

            let mut restored = prepared.psbt.0.clone();
            restored.inputs[1].tap_internal_key = None;
            restored.inputs[1].tap_merkle_root = None;
            restored.inputs[1].tap_scripts.clear();
            assert_eq!(restored, original);

            let oracle = ProgramOracle::new(root(), vec![]).unwrap();
            let signed = oracle.sign(prepared.clone()).unwrap();
            validate_program_response(&prepared, &signed, requirement.program.root()).unwrap();
            let repeated =
                prepare_program_request(&object, &requirement, signed.clone(), 1, vec![21])
                    .unwrap();
            assert_eq!(repeated.psbt.0, signed);
            let mut changed_transaction = signed;
            changed_transaction.unsigned_tx.output[0].value = Amount::from_sat(8_999);
            assert!(matches!(
                prepare_program_request(&object, &requirement, changed_transaction, 1, vec![21]),
                Err(ArtifactProgramError::Program(
                    ProgramError::ConflictingSignature
                ))
            ));
        }
    }

    #[test]
    fn only_recorded_program_paths_and_matching_funded_inputs_are_accepted() {
        let object = artifact(true);
        let requirement = requirement(&object, true);
        let original = funded(&object);
        let mut unrecorded = requirement.clone();
        unrecorded.path = ProgramSpendPath::ScriptPath(TapLeafHash::from_byte_array([3; 32]));
        assert!(matches!(
            prepare_program_request(&object, &unrecorded, original.clone(), 1, vec![]),
            Err(ArtifactProgramError::UnknownRequirement)
        ));
        unrecorded = requirement.clone();
        unrecorded.program = EmulatedProgram::new(
            requirement.program.instance().clone(),
            Xpub::from_priv(
                &Secp256k1::new(),
                &Xpriv::new_master(Network::Regtest, &[52; 32]).unwrap(),
            ),
        )
        .unwrap();
        assert!(matches!(
            prepare_program_request(&object, &unrecorded, original.clone(), 1, vec![]),
            Err(ArtifactProgramError::UnknownRequirement)
        ));
        assert!(matches!(
            prepare_program_request(&object, &requirement, original.clone(), 0, vec![]),
            Err(ArtifactProgramError::PrevoutMismatch)
        ));
        let mut underfunded = original.clone();
        underfunded.inputs[1].witness_utxo.as_mut().unwrap().value = Amount::from_sat(9_999);
        assert!(matches!(
            prepare_program_request(&object, &requirement, underfunded, 1, vec![]),
            Err(ArtifactProgramError::UnderfundedInput { available, required })
                if available == Amount::from_sat(9_999) && required == Amount::from_sat(10_000)
        ));
        let mut missing = original.clone();
        missing.inputs[0].witness_utxo = None;
        assert!(matches!(
            prepare_program_request(&object, &requirement, missing, 1, vec![]),
            Err(ArtifactProgramError::Program(ProgramError::MissingPrevout(
                0
            )))
        ));
        let mut malformed = original.clone();
        malformed.inputs.pop();
        assert!(matches!(
            prepare_program_request(&object, &requirement, malformed, 1, vec![]),
            Err(ArtifactProgramError::Program(ProgramError::Psbt(_)))
        ));
        assert!(matches!(
            prepare_program_request(&object, &requirement, original, 2, vec![]),
            Err(ArtifactProgramError::Program(ProgramError::InputIndex(2)))
        ));
    }

    #[test]
    fn contradictory_taproot_proofs_are_rejected_before_merging() {
        let object = artifact(true);
        let requirement = requirement(&object, true);
        let complete = prepare_program_request(&object, &requirement, funded(&object), 1, vec![])
            .unwrap()
            .psbt
            .0;
        let mut wrong_key = complete.clone();
        wrong_key.inputs[1].tap_internal_key =
            Some(root().to_keypair(&Secp256k1::new()).x_only_public_key().0);
        let mut wrong_root = complete.clone();
        wrong_root.inputs[1].tap_merkle_root = Some(TapNodeHash::from_byte_array([3; 32]));
        let mut wrong_script = complete.clone();
        wrong_script.inputs[1]
            .tap_scripts
            .values_mut()
            .next()
            .unwrap()
            .0 = ScriptBuf::from(vec![0x51]);
        let mut wrong_control = complete.clone();
        let (mut control, leaf) = wrong_control.inputs[1].tap_scripts.pop_first().unwrap();
        control.output_key_parity = control.output_key_parity ^ bitcoin::secp256k1::Parity::Odd;
        wrong_control.inputs[1].tap_scripts.insert(control, leaf);
        for conflicting in [wrong_key, wrong_root, wrong_script, wrong_control] {
            assert!(matches!(
                prepare_program_request(&object, &requirement, conflicting, 1, vec![]),
                Err(ArtifactProgramError::Program(
                    ProgramError::ConflictingTaprootMetadata
                ))
            ));
        }
    }
}
