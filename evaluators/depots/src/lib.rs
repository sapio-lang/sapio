//! Depot resolution predicates: CTV+CSFS conditions from the depot protocol
//! (jlspc, "Depots: Theft-Proof, Self-Custodial Bitcoin For Billions Of
//! Users", Figures 1 and 2), evaluated over the signed transaction view.
//!
//! One program serves every channel in a depot: guesses, salts, claim-template
//! hashes and the operator's blind guess signatures arrive as witness
//! evidence, so the depot's on-chain footprint never enumerates its users.
//!
//! Parameters (138 bytes):
//! `stage[1] || mode[1] || depot_id[32] || target_key[32] || guess_key[32]
//!  || prime_le[8] || expected_template_hash[32]`
//!
//! Stages: 0 = target reveal (Funding -> Targ), 1 = hit (Targ -> Hit),
//! 2 = claim (Hit output 1 -> Clm_Ui), 3 = double-hit burn (-> Burn).
//! Modes: 0 = operator-selected target (witness proves `t` with the
//! operator's target signature TaSig), 1 = default target (`t` = 0, no
//! target section; the paper's primed transactions).
//!
//! Witness layout: `[t_le[8] || tasig[64]]` (mode 0 only), then one guess
//! tuple for stages 1 and 2 or two tuples for stage 3, where a tuple is
//! `index_le[8] || salt[32] || guess_le[8] || gusig[64] || clm[32] ||
//! clm_prime[32]`. GuSig covers `<clm, clm_prime, index, salt, guess>`
//! under the depot's identity, so a spender can only bind the exact Claim
//! transaction the operator blind-signed at purchase time.
#![no_std]

#[path = "../../v2.rs"]
mod guest;
use core::ptr::addr_of_mut;
use guest::Arguments;
use sapio_covenant_fragments::{check_sig_from_stack, Context, Failure, XOnlyKey, MAX_VIEW_BYTES};

#[link(wasm_import_module = "sapio_crypto_v1")]
extern "C" {
    fn sha256(pointer: u32, length: u32, output: u32) -> i32;
}

fn hash(bytes: &[u8]) -> Result<[u8; 32], Failure> {
    let mut digest = [0; 32];
    let result = unsafe {
        sha256(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            digest.as_mut_ptr() as u32,
        )
    };
    match result {
        0 => Ok(digest),
        _ => Err(Failure::Crypto),
    }
}

static mut SCRATCH: [u8; MAX_VIEW_BYTES] = [0; MAX_VIEW_BYTES];

const PARAMETERS_LENGTH: usize = 138;
const TARGET_SECTION_LENGTH: usize = 8 + 64;
const TUPLE_LENGTH: usize = 8 + 32 + 8 + 64 + 32 + 32;

const STAGE_TARGET: u8 = 0;
const STAGE_HIT: u8 = 1;
const STAGE_CLAIM: u8 = 2;
const STAGE_BURN: u8 = 3;

const MODE_OPERATOR: u8 = 0;
const MODE_DEFAULT: u8 = 1;

#[no_mangle]
pub extern "C" fn sapio_evaluate_v2(
    program_pointer: u32,
    program_length: u32,
    parameters_pointer: u32,
    parameters_length: u32,
    view_pointer: u32,
    view_length: u32,
    witness_pointer: u32,
    witness_length: u32,
) -> i32 {
    guest::evaluate(
        [
            program_pointer,
            program_length,
            parameters_pointer,
            parameters_length,
            view_pointer,
            view_length,
            witness_pointer,
            witness_length,
        ],
        evaluate,
    )
}

struct Parameters<'a> {
    stage: u8,
    mode: u8,
    depot_id: &'a [u8; 32],
    target_key: XOnlyKey,
    guess_key: XOnlyKey,
    prime: u64,
    expected_template_hash: &'a [u8; 32],
}

impl<'a> Parameters<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, Failure> {
        if bytes.len() != PARAMETERS_LENGTH {
            return Err(Failure::InvalidEncoding);
        }
        let stage = bytes[0];
        let mode = bytes[1];
        if stage > STAGE_BURN || mode > MODE_DEFAULT {
            return Err(Failure::InvalidEncoding);
        }
        // The default target path needs no reveal predicate: the paper's
        // Targ' spend is a bare timelocked template equality.
        if stage == STAGE_TARGET && mode == MODE_DEFAULT {
            return Err(Failure::InvalidEncoding);
        }
        let prime = u64::from_le_bytes(bytes[98..106].try_into().unwrap());
        if prime == 0 {
            return Err(Failure::InvalidEncoding);
        }
        Ok(Self {
            stage,
            mode,
            depot_id: bytes[2..34].try_into().unwrap(),
            target_key: XOnlyKey::from_slice(&bytes[34..66])?,
            guess_key: XOnlyKey::from_slice(&bytes[66..98])?,
            prime,
            expected_template_hash: bytes[106..138].try_into().unwrap(),
        })
    }
}

struct GuessTuple<'a> {
    index: &'a [u8; 8],
    salt: &'a [u8; 32],
    guess: u64,
    guess_bytes: &'a [u8; 8],
    signature: &'a [u8],
    clm: &'a [u8; 32],
    clm_prime: &'a [u8; 32],
}

impl<'a> GuessTuple<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, Failure> {
        if bytes.len() != TUPLE_LENGTH {
            return Err(Failure::InvalidEncoding);
        }
        Ok(Self {
            index: bytes[..8].try_into().unwrap(),
            salt: bytes[8..40].try_into().unwrap(),
            guess: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            guess_bytes: bytes[40..48].try_into().unwrap(),
            signature: &bytes[48..112],
            clm: bytes[112..144].try_into().unwrap(),
            clm_prime: bytes[144..176].try_into().unwrap(),
        })
    }
}

fn tagged(tag: &[u8], data: &[&[u8]]) -> Result<[u8; 32], Failure> {
    let tag_hash = hash(tag)?;
    let mut preimage = [0u8; 64 + 32 + 32 + 32 + 8 + 32 + 8];
    let mut used = 0;
    for part in [&tag_hash[..], &tag_hash[..]].iter().chain(data.iter()) {
        let end = used + part.len();
        if end > preimage.len() {
            return Err(Failure::InvalidEncoding);
        }
        preimage[used..end].copy_from_slice(part);
        used = end;
    }
    hash(&preimage[..used])
}

/// `TaSig` message: the operator's commitment to the revealed target.
fn target_message(depot_id: &[u8; 32], t: &[u8; 8]) -> Result<[u8; 32], Failure> {
    tagged(b"depots/v1/target", &[depot_id, t])
}

/// `GuSig` message: the operator's blind commitment to one channel's claim
/// transactions, index, salt and guess.
fn guess_message(depot_id: &[u8; 32], tuple: &GuessTuple<'_>) -> Result<[u8; 32], Failure> {
    tagged(
        b"depots/v1/guess",
        &[
            depot_id,
            tuple.clm,
            tuple.clm_prime,
            tuple.index,
            tuple.salt,
            tuple.guess_bytes,
        ],
    )
}

fn evaluate(arguments: Arguments<'_>) -> Result<bool, Failure> {
    if !arguments.program.is_empty() {
        return Err(Failure::InvalidEncoding);
    }
    let parameters = Parameters::parse(arguments.parameters)?;
    let context = Context::parse_v2(arguments.view)?;

    // Establish the target `t`: proven with TaSig for an operator-selected
    // target, fixed at zero on the default path.
    let (target, witness) = match parameters.mode {
        MODE_OPERATOR => {
            let section = arguments
                .witness
                .get(..TARGET_SECTION_LENGTH)
                .ok_or(Failure::InvalidEncoding)?;
            let t_bytes: &[u8; 8] = section[..8].try_into().unwrap();
            let t = u64::from_le_bytes(*t_bytes);
            if t >= parameters.prime {
                return Ok(false);
            }
            let message = target_message(parameters.depot_id, t_bytes)?;
            if !check_sig_from_stack(&message, parameters.target_key, &section[8..])? {
                return Ok(false);
            }
            (t, &arguments.witness[TARGET_SECTION_LENGTH..])
        }
        _ => (0, arguments.witness),
    };

    // Validate one guess tuple: the operator's blind guess signature, and
    // whether the committed guess hits the established target.
    let check_tuple = |tuple: &GuessTuple<'_>| -> Result<bool, Failure> {
        let message = guess_message(parameters.depot_id, tuple)?;
        if !check_sig_from_stack(&message, parameters.guess_key, tuple.signature)? {
            return Ok(false);
        }
        Ok(tuple.guess < parameters.prime && tuple.guess == target)
    };

    // Scratch is independent of the immutable input arena and host state.
    let scratch = unsafe {
        core::slice::from_raw_parts_mut(addr_of_mut!(SCRATCH).cast::<u8>(), MAX_VIEW_BYTES)
    };
    let template_hash = context.template_hash(scratch)?;

    match parameters.stage {
        // Funding -> Targ: the operator reveals `t`; the child must be the
        // depot's fixed Target transaction.
        STAGE_TARGET => {
            if !witness.is_empty() {
                return Err(Failure::InvalidEncoding);
            }
            Ok(template_hash.as_bytes() == parameters.expected_template_hash)
        }
        // Targ -> Hit: any owner of a hit forces the fixed 50/50 split.
        STAGE_HIT => {
            let tuple = GuessTuple::parse(witness)?;
            Ok(check_tuple(&tuple)?
                && template_hash.as_bytes() == parameters.expected_template_hash)
        }
        // Hit output 1 -> Clm: the spending transaction must be the exact
        // claim transaction bound inside the operator's guess signature —
        // a CTV whose hash comes from the witness, authenticated by CSFS.
        STAGE_CLAIM => {
            let tuple = GuessTuple::parse(witness)?;
            let expected = match parameters.mode {
                MODE_OPERATOR => tuple.clm,
                _ => tuple.clm_prime,
            };
            Ok(check_tuple(&tuple)? && template_hash.as_bytes() == expected)
        }
        // Two distinct guess signatures for the same target burn the output:
        // the multi-hit griefing penalty, and the user's defense when the
        // operator equivocates with an arbitrary (Arb) spend.
        _ => {
            if witness.len() != 2 * TUPLE_LENGTH {
                return Err(Failure::InvalidEncoding);
            }
            let first = GuessTuple::parse(&witness[..TUPLE_LENGTH])?;
            let second = GuessTuple::parse(&witness[TUPLE_LENGTH..])?;
            if first.signature == second.signature {
                return Ok(false);
            }
            Ok(check_tuple(&first)?
                && check_tuple(&second)?
                && template_hash.as_bytes() == parameters.expected_template_hash)
        }
    }
}
