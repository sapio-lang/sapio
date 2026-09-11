//! A two-party eltoo research example using existing covenant fragments.
//!
//! Public compilation uses a fresh per-channel joint key. The disposable
//! fixture simulates joint authorization with one keypair; it is not MuSig2.
//! An external sponsor pays the complete fee without reducing channel funds.

use bitcoin::bip32::Xpub;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::key::TapTweak;
use bitcoin::psbt::{Input, Psbt};
use bitcoin::secp256k1::Keypair;
use bitcoin::secp256k1::{schnorr::Signature, Message, Secp256k1};
use bitcoin::sighash::{Prevouts, SighashCache};
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{
    taproot, Address, Amount, Network, OutPoint, ScriptBuf, TapSighashType, Transaction, TxOut,
    XOnlyPublicKey,
};
use emulator_connect::program::{ProgramSigningRequest, ProgramSpendPath, PSBT};
use miniscript::Descriptor;
use sapio::contract::abi::object::{ObjectMetadata, SupportedDescriptors};
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::*;
use sapio::*;
use sapio_base::fragments::{
    template_authorization_wasm_instance, template_hash, templatehash_wasm_instance, TemplateKey,
};
use sapio_base::policy::{PolicyCompiler, ScriptPolicy};
use sapio_base::program::EmulatedProgram;
use sapio_base::timelocks::{AbsTime, RelHeight};
use sapio_base::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::Arc;

pub mod recovery;

/// Errors in the example's public construction and signing helpers.
pub type Error = Box<dyn std::error::Error>;
/// State locks occupy a fixed historical timestamp interval, not block heights.
pub const LOCK_TIME_BASE: u32 = 500_000_000;
/// Last admissible state keeps every timestamp in that historical interval.
pub const MAX_STATE: u32 = 1_000_000;
/// Forty-byte OP_RETURN payload: this tag followed by the settlement leaf hash.
pub const RECOVERY_TAG: &[u8; 8] = b"eltoo/v1";
/// Declared sponsor contribution for suggested templates; rebinding may raise it.
pub const SUGGESTED_FEE: u64 = 2_000;

fn compilation(error: impl Display) -> CompilationError {
    CompilationError::Custom(error.to_string().into())
}

/// Public per-channel terms. Supply a new joint key for each channel.
#[derive(Clone, Debug, Serialize)]
pub struct Terms {
    joint_key: XOnlyPublicKey,
    capacity: u64,
    delay: u16,
    max_state: u32,
    alice: Address,
    bob: Address,
    update_program: EmulatedProgram,
}

/// One agreed allocation. Every public construction validates its domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// Monotonically increasing number within the channel's explicit interval.
    pub number: u32,
    /// Alice receives this amount; Bob receives the remaining channel capacity.
    pub alice_sats: u64,
}

impl Terms {
    /// Fix public authorization, payouts and the bounded state-number domain.
    pub fn new(
        joint_key: XOnlyPublicKey,
        oracle_root: Xpub,
        capacity: u64,
        delay: u16,
        max_state: u32,
        alice: Address,
        bob: Address,
    ) -> Result<Self, Error> {
        if capacity == 0 || capacity > 21_000_000 * 100_000_000 {
            return Err("channel capacity is outside the Bitcoin money range".into());
        }
        if delay == 0 {
            return Err("settlement delay must be positive".into());
        }
        if !(1..=MAX_STATE).contains(&max_state) {
            return Err("state interval must end between 1 and MAX_STATE".into());
        }
        let update_program = EmulatedProgram::new(
            template_authorization_wasm_instance(TemplateKey::InternalKey),
            oracle_root,
        )?;
        Ok(Self {
            joint_key,
            capacity,
            delay,
            max_state,
            alice,
            bob,
            update_program,
        })
    }

    pub fn joint_key(&self) -> XOnlyPublicKey {
        self.joint_key
    }
    pub fn capacity(&self) -> u64 {
        self.capacity
    }
    pub fn delay(&self) -> u16 {
        self.delay
    }
    pub fn max_state(&self) -> u32 {
        self.max_state
    }
    pub fn alice(&self) -> &Address {
        &self.alice
    }
    pub fn bob(&self) -> &Address {
        &self.bob
    }
    pub fn update_program(&self) -> &EmulatedProgram {
        &self.update_program
    }

    /// Map state numbers to timestamps already in the past, without a clock.
    pub fn lock_time(&self, number: u32) -> Result<u32, Error> {
        if !(1..=self.max_state).contains(&number) {
            return Err("state number lies outside this channel's interval".into());
        }
        Ok(LOCK_TIME_BASE + number)
    }

    fn validate(&self, state: State) -> Result<(), Error> {
        self.lock_time(state.number)?;
        if state.alice_sats > self.capacity {
            return Err("state allocation exceeds channel capacity".into());
        }
        Ok(())
    }

    /// Funding has an update leaf and an explicitly cooperative key path only.
    pub fn funding(&self) -> Channel {
        Channel {
            terms: self.clone(),
            state: None,
            settlement_program: None,
        }
    }

    /// Construct a state without compiling successors or contacting an oracle.
    pub fn state(&self, state: State) -> Result<Channel, Error> {
        self.validate(state)?;
        let hash = template_hash(&self.settlement_transaction(state)?, 0, None)?;
        let program = EmulatedProgram::new(
            templatehash_wasm_instance(hash),
            *self.update_program.root(),
        )?;
        Ok(Channel {
            terms: self.clone(),
            state: Some(state),
            settlement_program: Some(program),
        })
    }

    /// Canonical update template, with placeholder outpoints and fixed sequences.
    pub fn update_transaction(&self, state: State) -> Result<Transaction, Error> {
        let compiled = self.funding().compile_update(state)?;
        only_candidate(&compiled)
    }

    /// Settlement hash includes TL(n), so equal payouts in different states differ.
    pub fn settlement_transaction(&self, state: State) -> Result<Transaction, Error> {
        self.validate(state)?;
        let ctx = Context::new(
            Network::Regtest,
            Amount::from_sat(self.capacity),
            sapio_base::LoweringPlan::Native,
            "eltoo_settlement".try_into()?,
            Arc::new(Default::default()),
            None,
        );
        Ok(self.settlement_template(state, ctx)?.tx)
    }

    fn settlement_template(
        &self,
        state: State,
        ctx: Context,
    ) -> Result<sapio::template::Template, CompilationError> {
        self.validate(state).map_err(compilation)?;
        let alice = Compiled::from_address(self.alice.clone(), Amount::ZERO);
        let bob = Compiled::from_address(self.bob.clone(), Amount::ZERO);
        let mut template = ctx
            .template()
            .add_sequence()
            .set_sequence(0, RelHeight::from(self.delay).into())?
            .set_sequence(1, RelHeight::from(0).into())?
            .set_lock_time(
                AbsTime::try_from(self.lock_time(state.number).map_err(compilation)?)?.into(),
            )?
            .add_amount(Amount::from_sat(SUGGESTED_FEE))?;
        for (amount, destination) in [
            (state.alice_sats, &alice),
            (self.capacity - state.alice_sats, &bob),
        ] {
            if amount != 0 {
                template = template.add_output(Amount::from_sat(amount), destination, None)?;
            }
        }
        Ok(template.add_fees(Amount::from_sat(SUGGESTED_FEE))?.into())
    }

    /// Recover the actual lowered update script using only fixed terms and n.
    ///
    /// The dummy allocation affects only the discarded settlement leaf. Native
    /// update lowering depends on TL(n+1) and the shared program key alone.
    pub fn update_leaf_for(&self, number: u32) -> Result<ScriptBuf, Error> {
        self.state(State {
            number,
            alice_sats: self.capacity / 2,
        })?
        .update_leaf()
    }
}

/// Continuation arguments are candidate transactions, not changes to the guard.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub enum Candidate {
    Update(State),
    Settle,
}

/// One funding or state output, always retaining its public program source.
#[derive(Clone, Debug)]
pub struct Channel {
    terms: Terms,
    state: Option<State>,
    settlement_program: Option<EmulatedProgram>,
}

impl Channel {
    pub fn terms(&self) -> &Terms {
        &self.terms
    }
    pub fn current_state(&self) -> Option<State> {
        self.state
    }
    pub fn settlement_program(&self) -> Option<&EmulatedProgram> {
        self.settlement_program.as_ref()
    }

    #[guard(cached)]
    fn cooperative(self) {
        Clause::Key(self.terms.joint_key)
    }

    #[guard(policy, cached)]
    fn update_policy(self) -> Result<ScriptPolicy, CompilationError> {
        let program = self.terms.update_program.compile_policy()?;
        Ok(match self.state {
            None => program,
            Some(state) => ScriptPolicy::And(vec![
                Clause::try_from(AbsTime::try_from(LOCK_TIME_BASE + state.number + 1)?)?.into(),
                program,
            ]),
        })
    }

    #[guard(policy, cached)]
    fn settlement_policy(self) -> Result<ScriptPolicy, CompilationError> {
        Ok(match &self.settlement_program {
            None => Clause::Unsatisfiable.into(),
            Some(program) => ScriptPolicy::And(vec![
                Clause::try_from(RelHeight::from(self.terms.delay))?.into(),
                program.compile_policy()?,
            ]),
        })
    }

    #[compile_if]
    fn can_update(self, _ctx: Context) {
        if self
            .state
            .is_some_and(|state| state.number == self.terms.max_state)
        {
            ConditionalCompileType::Never
        } else {
            ConditionalCompileType::NoConstraint
        }
    }

    #[compile_if]
    fn can_settle(self, _ctx: Context) {
        if self.state.is_some() {
            ConditionalCompileType::NoConstraint
        } else {
            ConditionalCompileType::Never
        }
    }

    #[continuation(
        guarded_by = "[Self::update_policy]",
        compile_if = "[Self::can_update]",
        coerce_args = "Ok",
        web_api
    )]
    fn update(self, ctx: Context, candidate: Option<Candidate>) {
        let Some(Candidate::Update(state)) = candidate else {
            return empty();
        };
        if self.state.is_some_and(|old| state.number <= old.number) {
            return Err(compilation("updates must advance the state number"));
        }
        let successor = self.terms.state(state).map_err(compilation)?;
        let leaf = successor.settlement_leaf().map_err(compilation)?;
        let hash = TapLeafHash::from_script(&leaf, LeafVersion::TapScript);
        let mut payload = RECOVERY_TAG.to_vec();
        payload.extend_from_slice(hash.as_ref());
        let recovery = Compiled::from_op_return(&payload).map_err(compilation)?;
        ctx.template()
            .add_sequence()
            .set_sequence(0, RelHeight::from(0).into())?
            .set_sequence(1, RelHeight::from(0).into())?
            .set_lock_time(
                AbsTime::try_from(self.terms.lock_time(state.number).map_err(compilation)?)?.into(),
            )?
            .add_amount(Amount::from_sat(SUGGESTED_FEE))?
            .add_output(Amount::from_sat(self.terms.capacity), &successor, None)?
            .add_output(Amount::ZERO, &recovery, None)?
            .add_fees(Amount::from_sat(SUGGESTED_FEE))?
            .into()
    }

    #[continuation(
        guarded_by = "[Self::settlement_policy]",
        compile_if = "[Self::can_settle]",
        coerce_args = "Ok",
        web_api
    )]
    fn settle(self, ctx: Context, candidate: Option<Candidate>) {
        let Some(Candidate::Settle) = candidate else {
            return empty();
        };
        let state = self
            .state
            .ok_or_else(|| compilation("funding cannot settle"))?;
        Ok(Box::new(std::iter::once(
            self.terms.settlement_template(state, ctx),
        )))
    }

    fn compile_with(&self, action: Option<(&str, Candidate)>) -> Result<Compiled, Error> {
        let effects = match action {
            None => Default::default(),
            Some((name, candidate)) => serde_json::from_value(serde_json::json!({
                "effects": {format!("eltoo/@action/{name}/@suggested"): {"candidate":candidate}}
            }))?,
        };
        Ok(Compilable::compile(
            self,
            Context::new(
                Network::Regtest,
                Amount::from_sat(self.terms.capacity),
                sapio_base::LoweringPlan::Native,
                "eltoo".try_into()?,
                Arc::new(effects),
                None,
            ),
        )?)
    }

    /// Compile only the policy source, without adding candidate transitions.
    pub fn compile(&self) -> Result<Compiled, Error> {
        self.compile_with(None)
    }

    pub fn compile_update(&self, target: State) -> Result<Compiled, Error> {
        self.terms.validate(target)?;
        if self.state.is_some_and(|old| target.number <= old.number) {
            return Err("updates must advance the state number".into());
        }
        self.compile_with(Some(("update", Candidate::Update(target))))
    }

    pub fn compile_settlement(&self) -> Result<Compiled, Error> {
        if self.state.is_none() {
            return Err("funding has no settlement path".into());
        }
        self.compile_with(Some(("settle", Candidate::Settle)))
    }

    fn leaf_for(&self, program: &EmulatedProgram) -> Result<ScriptBuf, Error> {
        let compiled = self.compile()?;
        let key = program.derive_public_key()?.serialize();
        let input = spending_input(&compiled)?;
        let mut matches = input.tap_scripts.values().filter_map(|(script, _)| {
            script.instructions().any(|instruction| matches!(instruction,
                Ok(bitcoin::blockdata::script::Instruction::PushBytes(bytes)) if bytes.as_bytes() == key
            )).then_some(script.clone())
        });
        let script = matches
            .next()
            .ok_or("program leaf absent from this channel output")?;
        if matches.next().is_some() {
            return Err("ambiguous compiled program leaf".into());
        }
        Ok(script)
    }

    pub fn update_leaf(&self) -> Result<ScriptBuf, Error> {
        self.leaf_for(&self.terms.update_program)
    }

    pub fn settlement_leaf(&self) -> Result<ScriptBuf, Error> {
        self.leaf_for(
            self.settlement_program
                .as_ref()
                .ok_or("funding cannot settle")?,
        )
    }

    /// Populate the authenticated spending proof from actual compiler output.
    pub fn input(&self, coin: &Coin) -> Result<Input, Error> {
        let compiled = self.compile()?;
        if coin.txout.value.to_sat() != self.terms.capacity
            || coin.txout.script_pubkey != ScriptBuf::from(&compiled.address)
        {
            return Err("channel funding does not match the compiled output".into());
        }
        let mut input = spending_input(&compiled)?;
        input.witness_utxo = Some(coin.txout.clone());
        Ok(input)
    }
}

impl Contract for Channel {
    declare! {finish, Self::cooperative}
    declare! {updatable<Option<Candidate>>, Self::update, Self::settle}

    fn ensure_amount(&self, _ctx: Context) -> Result<Amount, CompilationError> {
        Ok(Amount::from_sat(self.terms.capacity))
    }

    fn pinned_internal_key(
        &self,
        _ctx: &Context,
    ) -> Result<Option<XOnlyPublicKey>, CompilationError> {
        Ok(Some(self.terms.joint_key))
    }

    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        let mut metadata = ObjectMetadata::default();
        metadata.extra.insert(
            "eltoo".into(),
            serde_json::json!({
                "terms": self.terms,
                "state": self.state,
                "settlement_program": self.settlement_program,
            }),
        );
        Ok(metadata)
    }
}

fn spending_input(compiled: &Compiled) -> Result<Input, Error> {
    let mut input = Input::default();
    match &compiled.descriptor {
        Some(SupportedDescriptors::XOnly(Descriptor::Tr(tree))) => {
            let info = tree.spend_info();
            input.tap_internal_key = Some(info.internal_key());
            input.tap_merkle_root = info.merkle_root();
            for leaf in info.leaves() {
                input.tap_scripts.insert(
                    leaf.control_block().clone(),
                    (leaf.script().to_owned(), leaf.leaf_version()),
                );
            }
        }
        Some(SupportedDescriptors::Taproot(tree)) => {
            let info = tree.spend_info();
            input.tap_internal_key = Some(info.internal_key());
            input.tap_merkle_root = info.merkle_root();
            for leaf in info.script_map().keys() {
                input.tap_scripts.insert(
                    info.control_block(leaf)
                        .ok_or("missing compiled control block")?,
                    leaf.clone(),
                );
            }
        }
        _ => return Err("expected a compiled Taproot descriptor".into()),
    }
    Ok(input)
}

fn only_candidate(compiled: &Compiled) -> Result<Transaction, Error> {
    if compiled.suggested_txs.len() != 1 || !compiled.ctv_to_tx.is_empty() {
        return Err("expected exactly one continuation candidate and no CTV branches".into());
    }
    Ok(compiled.suggested_txs.values().next().unwrap().tx.clone())
}

/// A funding assertion; the caller must obtain actual prevouts from its node.
#[derive(Clone, Debug)]
pub struct Coin {
    pub outpoint: OutPoint,
    pub txout: TxOut,
}

/// Replaceable key-path sponsor; its entire value becomes this transaction's fee.
#[derive(Clone, Debug)]
pub struct Sponsor {
    pub coin: Coin,
    pub internal_key: XOnlyPublicKey,
}

/// Bind an independently authorized template to authenticated channel and sponsor coins.
pub fn attach_inputs(
    mut transaction: Transaction,
    channel: Coin,
    mut input: Input,
    sponsor: Sponsor,
) -> Result<Psbt, Error> {
    if transaction.version != bitcoin::transaction::Version::TWO
        || transaction.input.len() != 2
        || sponsor.coin.txout.value.to_sat() == 0
    {
        return Err("eltoo requires version two, two inputs and a positive sponsor".into());
    }
    let script = Address::p2tr(
        &Secp256k1::new(),
        sponsor.internal_key,
        None,
        Network::Regtest,
    )
    .script_pubkey();
    if sponsor.coin.txout.script_pubkey != script {
        return Err("sponsor coin does not match its untweaked P2TR key".into());
    }
    let output_sum = transaction
        .output
        .iter()
        .try_fold(0u64, |sum, output| sum.checked_add(output.value.to_sat()))
        .ok_or("output sum overflows")?;
    if channel.txout.value.to_sat() != output_sum
        || input.witness_utxo.as_ref() != Some(&channel.txout)
    {
        return Err("channel capacity and authenticated prevout must match all outputs".into());
    }
    transaction.input[0].previous_output = channel.outpoint;
    transaction.input[1].previous_output = sponsor.coin.outpoint;
    input.non_witness_utxo = None;
    let mut psbt = Psbt::from_unsigned_tx(transaction)?;
    psbt.inputs[0] = input;
    psbt.inputs[1] = Input {
        witness_utxo: Some(sponsor.coin.txout),
        tap_internal_key: Some(sponsor.internal_key),
        ..Input::default()
    };
    Ok(psbt)
}

/// Jointly authorize a target template once, independent of its funding outpoints.
pub fn authorize_update(terms: &Terms, target: State, joint: &Keypair) -> Result<Signature, Error> {
    if joint.x_only_public_key().0 != terms.joint_key {
        return Err("authorization key does not match the channel's joint key".into());
    }
    let hash = template_hash(&terms.update_transaction(target)?, 0, None)?;
    Ok(Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref())?, joint))
}

pub fn update_request(
    source: &Channel,
    target: State,
    channel: Coin,
    sponsor: Sponsor,
    authorization: &Signature,
) -> Result<ProgramSigningRequest, Error> {
    let transaction = only_candidate(&source.compile_update(target)?)?;
    let input = source.input(&channel)?;
    let leaf = source.update_leaf()?;
    Ok(ProgramSigningRequest {
        instance: source.terms.update_program.instance().clone(),
        input_index: 0,
        witness: authorization.as_ref().to_vec(),
        path: ProgramSpendPath::ScriptPath(TapLeafHash::from_script(&leaf, LeafVersion::TapScript)),
        psbt: PSBT(attach_inputs(transaction, channel, input, sponsor)?),
    })
}

pub fn settlement_request(
    source: &Channel,
    channel: Coin,
    sponsor: Sponsor,
) -> Result<ProgramSigningRequest, Error> {
    let transaction = only_candidate(&source.compile_settlement()?)?;
    let state = source.state.ok_or("funding cannot settle")?;
    if transaction != source.terms.settlement_transaction(state)? {
        return Err("compiled settlement differs from the committed template".into());
    }
    let input = source.input(&channel)?;
    let leaf = source.settlement_leaf()?;
    Ok(ProgramSigningRequest {
        instance: source
            .settlement_program
            .as_ref()
            .unwrap()
            .instance()
            .clone(),
        input_index: 0,
        witness: vec![],
        path: ProgramSpendPath::ScriptPath(TapLeafHash::from_script(&leaf, LeafVersion::TapScript)),
        psbt: PSBT(attach_inputs(transaction, channel, input, sponsor)?),
    })
}

/// Add the sponsor's real SIGHASH_ALL signature after the oracle accepts input zero.
pub fn sign_sponsor(psbt: &mut Psbt, key: &Keypair) -> Result<(), Error> {
    if psbt.inputs.len() != 2 || psbt.inputs[1].tap_internal_key != Some(key.x_only_public_key().0)
    {
        return Err("sponsor signing key does not match input one".into());
    }
    let prevouts: Vec<_> = psbt
        .inputs
        .iter()
        .map(|input| {
            input
                .witness_utxo
                .as_ref()
                .ok_or("missing authenticated prevout")
        })
        .collect::<Result<_, _>>()?;
    let hash_ty = TapSighashType::All;
    let hash = SighashCache::new(&psbt.unsigned_tx).taproot_key_spend_signature_hash(
        1,
        &Prevouts::All(&prevouts),
        hash_ty,
    )?;
    let secp = Secp256k1::new();
    let key = key.tap_tweak(&secp, None).to_keypair();
    psbt.inputs[1].tap_key_sig = Some(taproot::Signature {
        signature: secp.sign_schnorr_no_aux_rand(&Message::from_digest_slice(hash.as_ref())?, &key),
        sighash_type: hash_ty,
    });
    Ok(())
}

/// Deterministic disposable test material, separate from public contract source.
pub mod fixture {
    use super::*;
    use bitcoin::bip32::Xpriv;
    use bitcoin::secp256k1::SecretKey;

    fn key(seed: u8) -> Keypair {
        Keypair::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[seed; 32]).unwrap(),
        )
    }
    pub fn joint_key() -> Keypair {
        key(101)
    }
    pub fn sponsor_key() -> Keypair {
        key(103)
    }
    pub fn oracle_key() -> Xpriv {
        Xpriv::new_master(Network::Testnet, &[102; 32]).unwrap()
    }
    pub fn terms() -> Terms {
        Terms::new(
            joint_key().x_only_public_key().0,
            Xpub::from_priv(&Secp256k1::new(), &oracle_key()),
            100_000,
            6,
            MAX_STATE,
            crate::program_example::recipient(104),
            crate::program_example::recipient(105),
        )
        .unwrap()
    }
    fn outpoint(tag: u8) -> OutPoint {
        OutPoint::new(
            bitcoin::Txid::from_byte_array(sha256::Hash::hash(&[tag]).to_byte_array()),
            0,
        )
    }
    pub fn coin(channel: &Channel, tag: u8) -> Coin {
        Coin {
            outpoint: outpoint(tag),
            txout: TxOut {
                value: bitcoin::Amount::from_sat(channel.terms.capacity),
                script_pubkey: (&channel.compile().unwrap().address).into(),
            },
        }
    }
    pub fn sponsor(value: u64, tag: u8) -> Sponsor {
        let internal_key = sponsor_key().x_only_public_key().0;
        Sponsor {
            coin: Coin {
                outpoint: outpoint(tag),
                txout: TxOut {
                    value: Amount::from_sat(value),
                    script_pubkey: Address::p2tr(
                        &Secp256k1::new(),
                        internal_key,
                        None,
                        Network::Regtest,
                    )
                    .script_pubkey(),
                },
            },
            internal_key,
        }
    }
}
