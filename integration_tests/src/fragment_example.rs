//! Compiled contract examples using physical and proven related-key fragments.
//!
//! These examples use disposable keys and synthetic funding. Public derivation
//! constructs the tweak proof; only the untweaked authorization key signs the
//! auxiliary TemplateHash message.

use crate::program_example::{recipient, FUNDING_SATS, PROGRAM_METADATA};
use bitcoin::bip32::Xpub;
use bitcoin::blockdata::{opcodes::all::OP_CHECKSIG, script::Builder};
use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::{Keypair, Message, Parity, Scalar, Secp256k1, SecretKey};
use bitcoin::taproot::{TapLeafHash, TapTweakHash};
use bitcoin::{Amount, Network, XOnlyPublicKey};
use emulator_connect::program::{ProgramSigningRequest, ProgramSpendPath, PSBT};
use sapio::contract::abi::object::ObjectMetadata;
use sapio::contract::*;
use sapio::*;
use sapio_base::effects::EffectPath;
use sapio_base::fragments::{
    known_tweak_witness, template_authorization_wasm_instance, template_hash, TemplateKey,
};
use sapio_base::program::{program_derivation_path, EmulatedProgram};
use sapio_base::Clause;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

/// Disposable participant key used by the physical-internal-key example.
pub fn participant_key() -> Keypair {
    Keypair::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[96; 32]).unwrap(),
    )
}

/// Which typed key fragment authorizes a candidate template.
#[derive(Clone, Copy, Debug)]
pub enum Authorization {
    /// Pin this participant as a separately authorized physical internal key.
    InternalKey(XOnlyPublicKey),
    /// Prove the public BIP32 root's additive relationship to the output key.
    KnownTweak,
}

/// One fixed spending policy with independently generated payment candidates.
pub struct FragmentContract {
    authorization: Authorization,
    program: EmulatedProgram,
}

impl FragmentContract {
    /// Construct public contract source without registering or contacting an oracle.
    pub fn new(authorization: Authorization, oracle_root: Xpub) -> Self {
        let key = match authorization {
            Authorization::InternalKey(_) => TemplateKey::InternalKey,
            Authorization::KnownTweak => TemplateKey::KnownTweak,
        };
        Self {
            authorization,
            program: EmulatedProgram::new(template_authorization_wasm_instance(key), oracle_root)
                .expect("valid disposable example root"),
        }
    }

    /// Retain the complete instance and public root for explicit signing requests.
    pub fn program(&self) -> &EmulatedProgram {
        &self.program
    }

    #[guard(policy, cached)]
    fn authorize(self) -> EmulatedProgram {
        self.program.clone()
    }

    #[guard(cached)]
    fn key_path(self) {
        Clause::Key(self.internal_key())
    }

    fn internal_key(&self) -> XOnlyPublicKey {
        match self.authorization {
            Authorization::InternalKey(key) => key,
            Authorization::KnownTweak => self
                .program
                .derive_public_key()
                .expect("validated public program root"),
        }
    }

    #[continuation(guarded_by = "[Self::authorize]", coerce_args = "Ok", web_api)]
    fn pay(self, ctx: Context, amount: Option<u64>) {
        let amount = amount.unwrap_or(6_000);
        let change = (FUNDING_SATS - 500)
            .checked_sub(amount)
            .ok_or(CompilationError::OutOfFunds)?;
        let recipient = Compiled::from_address(recipient(92), Amount::ZERO);
        let change_address =
            Compiled::from_address(crate::program_example::recipient(94), Amount::ZERO);
        ctx.template()
            .add_output(Amount::from_sat(amount), &recipient, None)?
            .add_output(Amount::from_sat(change), &change_address, None)?
            .add_fees(Amount::from_sat(500))?
            .into()
    }

    /// Compile a default candidate plus optional continuation effects.
    pub fn compile_candidates(&self, amounts: &[u64]) -> Result<Compiled, CompilationError> {
        let effects: BTreeMap<_, _> = amounts
            .iter()
            .enumerate()
            .map(|(index, amount)| (format!("candidate_{index}"), amount))
            .collect();
        let effects = serde_json::from_value(serde_json::json!({
            "effects": {"fragments/@action/pay/@suggested": effects}
        }))
        .expect("well-formed example effects");
        self.compile(Context::new(
            Network::Regtest,
            Amount::from_sat(FUNDING_SATS),
            sapio_base::LoweringPlan::Native,
            EffectPath::try_from("fragments").unwrap(),
            Arc::new(effects),
            None,
        ))
    }

    /// Authorize one candidate with an untweaked key and explicit annex.
    ///
    /// In known-tweak mode all derivation/opening arithmetic uses the public
    /// root. The private key argument only signs the TemplateHash message.
    pub fn signing_request(
        &self,
        mut psbt: Psbt,
        untweaked_authorizer: &Keypair,
        annex: Option<Vec<u8>>,
    ) -> Result<ProgramSigningRequest, Box<dyn Error>> {
        sapio_psbt::annex::set(&mut psbt.inputs[0], annex)?;
        let hash = template_hash(
            &psbt.unsigned_tx,
            0,
            sapio_psbt::annex::get(&psbt.inputs[0])?,
        )?;
        let secp = Secp256k1::new();
        let signature = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest_slice(hash.as_ref())?,
            untweaked_authorizer,
        );
        let (path, witness) = match self.authorization {
            Authorization::InternalKey(_) => {
                let key = self.program.derive_public_key()?;
                let script = Builder::new()
                    .push_slice(&key.serialize())
                    .push_opcode(OP_CHECKSIG)
                    .into_script();
                let (_, (script, version)) = psbt.inputs[0]
                    .tap_scripts
                    .iter()
                    .find(|(_, (candidate, _))| candidate == &script)
                    .ok_or("compiled program has no matching key leaf")?;
                (
                    ProgramSpendPath::ScriptPath(TapLeafHash::from_script(script, *version)),
                    signature.as_ref().to_vec(),
                )
            }
            Authorization::KnownTweak => {
                let root = *self.program.root();
                let (root_key, root_parity) = root.public_key.x_only_public_key();
                let mut child = root;
                let mut cumulative = Scalar::ZERO;
                for index in program_derivation_path(self.program.instance().id()) {
                    let (tweak, _) = child.ckd_pub_tweak(index)?;
                    cumulative = add(cumulative, Scalar::from(tweak));
                    child = child.ckd_pub(&secp, index)?;
                }
                let (internal, child_parity) = child.public_key.x_only_public_key();
                let tap =
                    TapTweakHash::from_key_and_tweak(internal, psbt.inputs[0].tap_merkle_root)
                        .to_scalar();
                let (_, output_parity) = internal.add_tweak(&secp, &tap)?;
                // Full child = root + C*G. Account for both x-only lifts;
                // choose the output representative that leaves P positive.
                let output_sign = root_parity ^ child_parity;
                let tweak = add(
                    with_sign(cumulative, root_parity),
                    with_sign(tap, output_sign),
                );
                let parity = output_parity ^ output_sign;
                let witness = known_tweak_witness(root_key, tweak, parity, Some(&signature));
                // Neither the proof nor key-path signing needs a control block
                // or claimed internal key; the retained Merkle root defines
                // the final Taproot tweak.
                psbt.inputs[0].tap_scripts.clear();
                psbt.inputs[0].tap_key_origins.clear();
                psbt.inputs[0].tap_internal_key = None;
                (ProgramSpendPath::KeyPath, witness)
            }
        };
        Ok(ProgramSigningRequest {
            instance: self.program.instance().clone(),
            input_index: 0,
            witness,
            path,
            psbt: PSBT(psbt),
        })
    }
}

impl Contract for FragmentContract {
    declare! {finish, Self::key_path}
    declare! {updatable<Option<u64>>, Self::pay}

    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.internal_key()))
    }

    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        let mut metadata = ObjectMetadata::default();
        metadata.extra.insert(
            PROGRAM_METADATA.into(),
            serde_json::to_value(&self.program).map_err(CompilationError::SerializationError)?,
        );
        Ok(metadata)
    }
}

fn negate(value: Scalar) -> Scalar {
    if value == Scalar::ZERO {
        value
    } else {
        Scalar::from(
            SecretKey::from_slice(&value.to_be_bytes())
                .unwrap()
                .negate(),
        )
    }
}

fn add(left: Scalar, right: Scalar) -> Scalar {
    if left == Scalar::ZERO {
        right
    } else if negate(left) == right {
        Scalar::ZERO
    } else {
        Scalar::from(
            SecretKey::from_slice(&left.to_be_bytes())
                .unwrap()
                .add_tweak(&right)
                .expect("canonical scalars with a nonzero sum"),
        )
    }
}

fn with_sign(value: Scalar, parity: Parity) -> Scalar {
    if parity == Parity::Odd {
        negate(value)
    } else {
        value
    }
}
