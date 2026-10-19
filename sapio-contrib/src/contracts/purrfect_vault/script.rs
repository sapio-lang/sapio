//! The upstream Purrfect Vault Bitcoin Script programs, including OP_CAT.
//!
//! Ported with canonical signature-split guards from Taproot Wizards' revision
//! `9e41e4dc0a7dc5b6267d117eb8b60cbf594e02ff`.

/*
Copyright 2024 Taproot Wizards

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
*/

use super::{Stage, WITHDRAWAL_DELAY};

const TAPSIGHASH_TAG: &[u8; 10] = b"TapSighash";
const BIP0340_CHALLENGE_TAG: &[u8; 17] = b"BIP0340/challenge";
const DUST_AMOUNT: &[u8; 8] = &546_u64.to_le_bytes();
const G_X: &[u8; 32] = &[
    0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0, 0x62, 0x95, 0xce, 0x87, 0x0b, 0x07,
    0x02, 0x9b, 0xfc, 0xdb, 0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b, 0x16, 0xf8, 0x17, 0x98,
];

/// Construct the upstream leaf with canonical signature-split guards.
pub fn script(stage: Stage, owner: bitcoin::XOnlyPublicKey) -> bitcoin::ScriptBuf {
    match stage {
        Stage::Trigger => vault_trigger_withdrawal(owner),
        Stage::Cancel => vault_cancel_withdrawal(owner),
        Stage::Complete => vault_complete_withdrawal(owner),
    }
}

use bitcoin::opcodes::all::{
    OP_2DUP, OP_CAT, OP_CHECKSIG, OP_CHECKSIGVERIFY, OP_CSV, OP_DROP, OP_DUP, OP_EQUALVERIFY,
    OP_FROMALTSTACK, OP_GREATERTHAN, OP_HASH256, OP_LESSTHAN, OP_ROT, OP_SHA256, OP_SIZE, OP_SWAP,
    OP_TOALTSTACK, OP_VERIFY,
};
use bitcoin::script::Builder;
use bitcoin::{ScriptBuf, Sequence, XOnlyPublicKey};

// Fixed widths make each CAT preimage unambiguous. Zero denotes a variable
// script: the vault field is fixed at the P2TR serialization's 35 bytes, so
// the concatenated script hash fixes the remaining sponsor script exactly.
fn prefix(stage: Stage, owner: XOnlyPublicKey) -> Builder {
    let mut builder = Builder::new()
        .push_x_only_key(&owner)
        .push_opcode(OP_CHECKSIGVERIFY);
    if stage == Stage::Complete {
        builder = builder
            .push_sequence(Sequence::from_height(WITHDRAWAL_DELAY))
            .push_opcode(OP_CSV)
            .push_opcode(OP_DROP);
    }
    // Top to bottom, excluding the owner signature consumed above. The first
    // three fields are the synthetic signature's increment, last byte, prefix.
    let fields: &[usize] = match stage {
        Stage::Trigger => &[1, 1, 63, 0, 8, 35, 8, 0, 4, 1, 32, 1, 32, 32, 4, 4, 1, 1],
        Stage::Cancel => &[1, 1, 63, 0, 8, 35, 8, 4, 1, 32, 1, 32, 32, 4, 4, 1, 1],
        Stage::Complete => &[
            1, 1, 63, 36, 0, 8, 35, 4, 4, 36, 4, 36, 4, 4, 1, 32, 4, 1, 32, 32, 32, 4, 4, 1, 1,
        ],
    };
    for &width in fields {
        if width != 0 {
            builder = builder
                .push_opcode(OP_SIZE)
                .push_int(width as i64)
                .push_opcode(OP_EQUALVERIFY);
        }
        builder = builder.push_opcode(OP_TOALTSTACK);
    }
    for _ in fields {
        builder = builder.push_opcode(OP_FROMALTSTACK);
    }
    builder
}

// For 1..127 raw script bytes, minimal Script-number encoding is exactly the
// one-byte CompactSize prefix. Construct it in Script rather than trusting an
// evidence-supplied serialization that might include additional outputs.
fn serialize_destination(builder: Builder) -> Builder {
    builder
        .push_opcode(OP_SIZE)
        .push_opcode(OP_DUP)
        .push_int(0)
        .push_opcode(OP_GREATERTHAN)
        .push_opcode(OP_VERIFY)
        .push_opcode(OP_DUP)
        .push_int(128)
        .push_opcode(OP_LESSTHAN)
        .push_opcode(OP_VERIFY)
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT)
}

pub(crate) fn vault_trigger_withdrawal(x_only_pubkey: XOnlyPublicKey) -> ScriptBuf {
    let mut builder = prefix(Stage::Trigger, x_only_pubkey);
    // The witness program needs to have the signature components except the outputs and the pre_scriptpubkeys and pre_amounts,
    // followed by the target scriptpubkey (the amount for that output will be fixed)
    // followed by the vault output amount, then the vault scriptpubkey,
    // followed by the fee amount, then the fee-paying scriptpubkey
    // followed by the mangled signature
    // and finally the a normal signature that signs with vault pubkey
    builder = builder
        .push_opcode(OP_TOALTSTACK) // incremented challenge byte
        .push_opcode(OP_TOALTSTACK) // original challenge byte
        .push_opcode(OP_TOALTSTACK) // generator R and first 31 challenge bytes
        .push_opcode(OP_TOALTSTACK) // push the fee-paying scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the fee amount to the alt stack
        .push_opcode(OP_2DUP) // make a second copy of the vault scriptpubkey and amount so we can check input = output
        .push_opcode(OP_TOALTSTACK) // push the first copy of the vault scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the first copy of the vault amount to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the second copy of the vault scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the second copy of the vault amount to the alt stack
        ;
    builder = serialize_destination(builder)
        .push_opcode(OP_TOALTSTACK) // canonical destination serialization
        // start with encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_slice([0x00u8, 0x00u8, 0x00u8, 0x00u8]) // add input index of 0
        .push_opcode(OP_SWAP) // bring working sigmsg back to top of stack
        .push_opcode(OP_CAT) // input index
        .push_opcode(OP_CAT) // spend type
        .push_slice(*DUST_AMOUNT) // push the dust amount for the target output
        .push_opcode(OP_FROMALTSTACK) // get the target scriptpubkey
        .push_opcode(OP_CAT) // cat the dust amount and the target scriptpubkey
        .push_opcode(OP_FROMALTSTACK) // get the output amount
        .push_opcode(OP_FROMALTSTACK) // get the second copy of the scriptpubkey
        .push_opcode(OP_CAT) // cat the output amount and the second copy of the scriptpubkey
        .push_opcode(OP_SWAP) // put the outputs in the right order (vault then target)
        .push_opcode(OP_CAT) // cat the vault output and target output together
        .push_opcode(OP_SHA256) // hash the output
        .push_opcode(OP_SWAP) // move the hashed encoded outputs below our working sigmsg
        .push_opcode(OP_CAT) // outputs
        .push_opcode(OP_CAT) // prev sequences
        .push_opcode(OP_FROMALTSTACK) // get the other copy of the vault amount
        .push_opcode(OP_FROMALTSTACK) // get the other copy of the vault scriptpubkey
        .push_opcode(OP_FROMALTSTACK) // get the fee amount
        .push_opcode(OP_FROMALTSTACK) // get the fee-paying scriptpubkey
        .push_opcode(OP_SWAP) // move the fee-paying scriptpubkey below the fee amount
        .push_opcode(OP_TOALTSTACK) // move fee amount to alt stack
        .push_opcode(OP_CAT) // cat the vault scriptpubkey fee-paying scriptpubkey
        .push_opcode(OP_SWAP) // move the vault amount to the top of the stack
        .push_opcode(OP_TOALTSTACK) // move the vault amount to the alt stack
        .push_opcode(OP_SHA256) // hash the scriptpubkeys, should now be consensus encoding
        .push_opcode(OP_SWAP) // move the hashed encoded scriptpubkeys below our working sigmsg
        .push_opcode(OP_CAT) // prev scriptpubkeys
        .push_opcode(OP_FROMALTSTACK) // get the vault amount
        .push_opcode(OP_FROMALTSTACK) // get the fee amount
        .push_opcode(OP_CAT) // cat the vault amount and the fee amount
        .push_opcode(OP_SHA256) // hash the amounts
        .push_opcode(OP_SWAP) // move the hashed encoded amounts below our working sigmsg
        .push_opcode(OP_CAT) // prev amounts
        .push_opcode(OP_CAT) // prevouts
        .push_opcode(OP_CAT) // lock time
        .push_opcode(OP_CAT) // version
        .push_opcode(OP_CAT) // control
        .push_opcode(OP_CAT); // epoch

    builder = add_signature_construction_and_check(builder);
    builder.into_script()
}

pub(crate) fn vault_complete_withdrawal(x_only_pubkey: XOnlyPublicKey) -> ScriptBuf {
    let mut builder = prefix(Stage::Complete, x_only_pubkey);
    // The witness program needs to have the signature components except the outputs, prevouts,
    // followed by the previous transaction version, inputs, and locktime
    // followed by vault SPK, the vault amount, and the target SPK
    // followed by the fee-paying txout
    // followed by the mangled signature
    // and finally the a normal signature that signs with vault pubkey
    builder = builder
        .push_opcode(OP_TOALTSTACK) // incremented challenge byte
        .push_opcode(OP_TOALTSTACK) // original challenge byte
        .push_opcode(OP_TOALTSTACK) // generator R and first 31 challenge bytes
        .push_opcode(OP_TOALTSTACK); // fee-paying outpoint
    builder = serialize_destination(builder)
        .push_opcode(OP_DUP) // make a second copy of the target scriptpubkey so we can use it later
        .push_opcode(OP_TOALTSTACK) // push the target scriptpubkey to the alt stack
        .push_slice(*DUST_AMOUNT) // push the dust amount for the previous tx second output
        .push_opcode(OP_SWAP) // swap the dust amount to the top of the stack
        .push_opcode(OP_CAT) // consensus-encode the second output for the previous TX
        .push_opcode(OP_SWAP) // get the vault amount to the top of the stack
        .push_opcode(OP_DUP) // make a second copy of the vault amount so we can use it later
        .push_opcode(OP_FROMALTSTACK) // get the target scriptpubkey
        .push_opcode(OP_CAT) // cat the target scriptpubkey and the vault amount.
        .push_opcode(OP_SHA256) // hash the target SPK + vault amount, this is our encoded output commitment
        .push_opcode(OP_TOALTSTACK) // move the output commitment to the alt stack
        .push_opcode(OP_SWAP) // get the vault amount to the second position on the stack
        .push_opcode(OP_ROT) // move the vault address to the top of the stack
        .push_opcode(OP_SWAP) // move the second output from the previous TX to the top of the stack
        .push_opcode(OP_CAT) // cat the vault amount and the second output from the previous TX
        .push_opcode(OP_CAT) // cat the vault address, now have all the outputs from the previous TX
        .push_int(2) // add the number of outputs from the previous TX
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT) // cat the outputs with their count from the previous TX
        .push_opcode(OP_SWAP) // move the outputs down, and the previous TX locktime to the top of the stack
        .push_opcode(OP_CAT) // cat the previous TX locktime with the outputs
        .push_opcode(OP_CAT) // second sequence and outputs
        .push_slice([0]) // second input's empty scriptSig
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT) // second outpoint
        .push_opcode(OP_CAT) // first sequence
        .push_slice([0]) // first input's empty scriptSig
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT) // first outpoint
        .push_int(2) // canonical input count
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT) // previous transaction version
        .push_opcode(OP_HASH256) // hash the whole thing twice to get the TXID
        .push_opcode(OP_FROMALTSTACK) // get the output commitment
        .push_opcode(OP_SWAP) // move the output commitment below the TXID
        .push_opcode(OP_TOALTSTACK) // move the TXID to the alt stack
        .push_opcode(OP_TOALTSTACK) // move the output commitment to the alt stack
        // start with encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_opcode(OP_CAT) // input index
        .push_opcode(OP_CAT) // spend type
        .push_opcode(OP_FROMALTSTACK) // get the output commitment
        .push_opcode(OP_SWAP) // move the output commitment below our working sigmsg
        .push_opcode(OP_CAT) // outputs
        .push_opcode(OP_CAT) // prev sequences
        .push_opcode(OP_CAT) // prev scriptpubkeys
        .push_opcode(OP_CAT) // prev amounts
        .push_opcode(OP_FROMALTSTACK) // get the previous TXID from the alt stack
        .push_slice([0x00u8, 0x00u8, 0x00u8, 0x00u8]) // add the output index for the previous TX
        .push_opcode(OP_FROMALTSTACK) // get the fee-paying txout
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT) // smoosh the fee-paying txout with the previous TXID and output index
        .push_opcode(OP_SHA256) // hash the whole thing to get the prevout commitment
        .push_opcode(OP_SWAP) // move the hashed prevout commitment below our working sigmsg
        .push_opcode(OP_CAT) // prevouts
        .push_opcode(OP_CAT) // lock time
        .push_opcode(OP_CAT) // version
        .push_opcode(OP_CAT) // control
        .push_opcode(OP_CAT); // epoch
    builder = add_signature_construction_and_check(builder);
    builder.into_script()
}

pub(crate) fn vault_cancel_withdrawal(x_only_pubkey: XOnlyPublicKey) -> ScriptBuf {
    let mut builder = prefix(Stage::Cancel, x_only_pubkey);
    // The witness program needs to have the signature components except the outputs and the pre_scriptpubkeys and pre_amounts,
    // followed by the output amount, then the script pubkey,
    // followed by the fee amount, then the fee-paying scriptpubkey
    // followed by the mangled signature
    // and finally the a normal signature that signs with vault pubkey
    builder = builder
        .push_opcode(OP_TOALTSTACK) // incremented challenge byte
        .push_opcode(OP_TOALTSTACK) // original challenge byte
        .push_opcode(OP_TOALTSTACK) // generator R and first 31 challenge bytes
        .push_opcode(OP_TOALTSTACK) // push the fee-paying scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the fee amount to the alt stack
        .push_opcode(OP_2DUP) // make a second copy of the vault scriptpubkey and amount so we can check input = output
        .push_opcode(OP_TOALTSTACK) // push the first copy of the vault scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the first copy of the vault amount to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the second copy of the vault scriptpubkey to the alt stack
        .push_opcode(OP_TOALTSTACK) // push the second copy of the vault amount to the alt stack
        // start with encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_opcode(OP_CAT) // encoded leaf hash
        .push_slice([0x00u8, 0x00u8, 0x00u8, 0x00u8]) // add input index of 0
        .push_opcode(OP_SWAP) // bring working sigmsg back to top of stack
        .push_opcode(OP_CAT) // input index
        .push_opcode(OP_CAT) // spend type
        .push_opcode(OP_FROMALTSTACK) // get the output amount
        .push_opcode(OP_FROMALTSTACK) // get the second copy of the scriptpubkey
        .push_opcode(OP_CAT) // cat the output amount and the second copy of the scriptpubkey
        .push_opcode(OP_SHA256) // hash the output
        .push_opcode(OP_SWAP) // move the hashed encoded outputs below our working sigmsg
        .push_opcode(OP_CAT) // outputs
        .push_opcode(OP_CAT) // prev sequences
        .push_opcode(OP_FROMALTSTACK) // get the other copy of the vault amount
        .push_opcode(OP_FROMALTSTACK) // get the other copy of the vault scriptpubkey
        .push_opcode(OP_FROMALTSTACK) // get the fee amount
        .push_opcode(OP_FROMALTSTACK) // get the fee-paying scriptpubkey
        .push_opcode(OP_SWAP) // move the fee-paying scriptpubkey below the fee amount
        .push_opcode(OP_TOALTSTACK) // move fee amount to alt stack
        .push_opcode(OP_CAT) // cat the vault scriptpubkey fee-paying scriptpubkey
        .push_opcode(OP_SWAP) // move the vault amount to the top of the stack
        .push_opcode(OP_TOALTSTACK) // move the vault amount to the alt stack
        .push_opcode(OP_SHA256) // hash the scriptpubkeys, should now be consensus encoding
        .push_opcode(OP_SWAP) // move the hashed encoded scriptpubkeys below our working sigmsg
        .push_opcode(OP_CAT) // prev scriptpubkeys
        .push_opcode(OP_FROMALTSTACK) // get the vault amount
        .push_opcode(OP_FROMALTSTACK) // get the fee amount
        .push_opcode(OP_CAT) // cat the vault amount and the fee amount
        .push_opcode(OP_SHA256) // hash the amounts
        .push_opcode(OP_SWAP) // move the hashed encoded amounts below our working sigmsg
        .push_opcode(OP_CAT) // prev amounts
        .push_opcode(OP_CAT) // prevouts
        .push_opcode(OP_CAT) // lock time
        .push_opcode(OP_CAT) // version
        .push_opcode(OP_CAT) // control
        .push_opcode(OP_CAT); // epoch
    builder = add_signature_construction_and_check(builder);
    builder.into_script()
}

/// Assumes that the builder has the sigmsg on the stack, and the pre-computed mangled signature on top of the alt stack.
/// will construct the tagged hash and the signature and do the verification
/// Call this after you've CAT'd the epoch onto the sigmsg
pub(crate) fn add_signature_construction_and_check(builder: Builder) -> Builder {
    builder
        .push_slice(*TAPSIGHASH_TAG) // push tag
        .push_opcode(OP_SHA256) // hash tag
        .push_opcode(OP_DUP) // dup hash
        .push_opcode(OP_ROT) // move the sighash to the top of the stack
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_SHA256) // tagged hash of the sighash
        .push_slice(*BIP0340_CHALLENGE_TAG) // push tag
        .push_opcode(OP_SHA256)
        .push_opcode(OP_DUP)
        .push_opcode(OP_ROT) // bring challenge to the top of the stack
        .push_slice(*G_X) // G is used for the pubkey and K
        .push_opcode(OP_DUP)
        .push_opcode(OP_DUP)
        .push_opcode(OP_DUP)
        .push_opcode(OP_TOALTSTACK) // we'll need a copy of G later to be our R value in the signature
        .push_opcode(OP_TOALTSTACK) // we'll need a copy of G later to be our R value in the signature
        .push_opcode(OP_ROT) // bring the challenge to the top of the stack
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT)
        .push_opcode(OP_CAT) // cat the two tags, R, P, and M values together
        .push_opcode(OP_SHA256) // hash the whole thing to get the s value for the signature
        .push_opcode(OP_FROMALTSTACK) // bring G back from the alt stack to use as the R value in the signature
        .push_opcode(OP_SWAP)
        .push_opcode(OP_CAT) // cat the R value with the s value for a complete signature
        .push_opcode(OP_FROMALTSTACK) // bring G back from the alt stack to use as the R value in the signature
        .push_opcode(OP_FROMALTSTACK) // grab the pre-computed signature minus the last byte from the alt stack
        .push_opcode(OP_ROT) // Move the G value to the bottom of the stack
        .push_opcode(OP_SWAP) // put the pre-computed signature on the top of the stack
        .push_opcode(OP_DUP) // we'll need a second copy later to do the actual signature verification
        .push_opcode(OP_FROMALTSTACK) // grab the last byte of the signature hash from the alt stack
        .push_opcode(OP_CAT)
        .push_opcode(OP_ROT) // bring the script-computed signature to the top of the stack
        .push_opcode(OP_EQUALVERIFY) // check that the script-computed and pre-computed signatures match
        .push_opcode(OP_FROMALTSTACK) // grab the last byte of the signature from the alt stack, should be +1 from the pre-computed signature
        .push_opcode(OP_CAT)
        .push_opcode(OP_SWAP) // bring G to the top of the stack
        .push_opcode(OP_CHECKSIG)
}
