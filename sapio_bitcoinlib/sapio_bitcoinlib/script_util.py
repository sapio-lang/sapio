#!/usr/bin/env python3
# Copyright (c) 2019 The Bitcoin Core developers
# Distributed under the MIT software license, see the accompanying
# file COPYING or http://www.opensource.org/licenses/mit-license.php.
"""Useful Script constants and utils."""
import struct

from .messages import CTransaction, CTxOut, uint256_from_str, ser_uint256, ser_string
from .hash_functions import hash256
from .script import (
    CScript,
    FindAndDelete,
    OP_CODESEPARATOR,
    SIGHASH_NONE,
    SIGHASH_SINGLE,
    SIGHASH_ANYONECANPAY,
)

# To prevent a "tx-size-small" policy rule error, a transaction has to have a
# non-witness size of at least 82 bytes (MIN_STANDARD_TX_NONWITNESS_SIZE in
# src/policy/policy.h). Considering a Tx with the smallest possible single
# input (blank, empty scriptSig), and with an output omitting the scriptPubKey,
# we get to a minimum size of 60 bytes:
#
# Tx Skeleton: 4 [Version] + 1 [InCount] + 1 [OutCount] + 4 [LockTime] = 10 bytes
# Blank Input: 32 [PrevTxHash] + 4 [Index] + 1 [scriptSigLen] + 4 [SeqNo] = 41 bytes
# Output:      8 [Amount] + 1 [scriptPubKeyLen] = 9 bytes
#
# Hence, the scriptPubKey of the single output has to have a size of at
# least 22 bytes, which corresponds to the size of a P2WPKH scriptPubKey.
# The following script constant consists of a single push of 21 bytes of 'a':
#   <PUSH_21> <21-bytes of 'a'>
# resulting in a 22-byte size. It should be used whenever (small) fake
# scriptPubKeys are needed, to guarantee that the minimum transaction size is
# met.
DUMMY_P2WPKH_SCRIPT = CScript([b"a" * 21])


# TODO: Allow cached hashPrevouts/hashSequence/hashOutputs to be provided.
# Performance optimization probably not necessary for python tests, however.
# Note that this corresponds to sigversion == 1 in EvalScript, which is used
# for version 0 witnesses.
def LegacySignatureHash(script, txTo, inIdx, hashtype):
    """Consensus-correct SignatureHash

    Returns (hash, err) to precisely match the consensus-critical behavior of
    the SIGHASH_SINGLE bug. (inIdx is *not* checked for validity)
    """
    HASH_ONE = b"\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"

    if inIdx >= len(txTo.vin):
        return (HASH_ONE, "inIdx %d out of range (%d)" % (inIdx, len(txTo.vin)))
    txtmp = CTransaction(txTo)

    for txin in txtmp.vin:
        txin.scriptSig = b""
    txtmp.vin[inIdx].scriptSig = FindAndDelete(script, CScript([OP_CODESEPARATOR]))

    if (hashtype & 0x1F) == SIGHASH_NONE:
        txtmp.vout = []

        for i in range(len(txtmp.vin)):
            if i != inIdx:
                txtmp.vin[i].nSequence = 0

    elif (hashtype & 0x1F) == SIGHASH_SINGLE:
        outIdx = inIdx
        if outIdx >= len(txtmp.vout):
            return (HASH_ONE, "outIdx %d out of range (%d)" % (outIdx, len(txtmp.vout)))

        tmp = txtmp.vout[outIdx]
        txtmp.vout = []
        for i in range(outIdx):
            txtmp.vout.append(CTxOut(-1))
        txtmp.vout.append(tmp)

        for i in range(len(txtmp.vin)):
            if i != inIdx:
                txtmp.vin[i].nSequence = 0

    if hashtype & SIGHASH_ANYONECANPAY:
        tmp = txtmp.vin[inIdx]
        txtmp.vin = []
        txtmp.vin.append(tmp)

    s = txtmp.serialize_without_witness()
    s += struct.pack(b"<I", hashtype)

    hash = hash256(s)

    return (hash, None)


def SegwitV0SignatureHash(script, txTo, inIdx, hashtype, amount):

    hashPrevouts = 0
    hashSequence = 0
    hashOutputs = 0

    if not (hashtype & SIGHASH_ANYONECANPAY):
        serialize_prevouts = bytes()
        for i in txTo.vin:
            serialize_prevouts += i.prevout.serialize()
        hashPrevouts = uint256_from_str(hash256(serialize_prevouts))

    if (
        not (hashtype & SIGHASH_ANYONECANPAY)
        and (hashtype & 0x1F) != SIGHASH_SINGLE
        and (hashtype & 0x1F) != SIGHASH_NONE
    ):
        serialize_sequence = bytes()
        for i in txTo.vin:
            serialize_sequence += struct.pack("<I", i.nSequence)
        hashSequence = uint256_from_str(hash256(serialize_sequence))

    if (hashtype & 0x1F) != SIGHASH_SINGLE and (hashtype & 0x1F) != SIGHASH_NONE:
        serialize_outputs = bytes()
        for o in txTo.vout:
            serialize_outputs += o.serialize()
        hashOutputs = uint256_from_str(hash256(serialize_outputs))
    elif (hashtype & 0x1F) == SIGHASH_SINGLE and inIdx < len(txTo.vout):
        serialize_outputs = txTo.vout[inIdx].serialize()
        hashOutputs = uint256_from_str(hash256(serialize_outputs))

    ss = bytes()
    ss += struct.pack("<i", txTo.nVersion)
    ss += ser_uint256(hashPrevouts)
    ss += ser_uint256(hashSequence)
    ss += txTo.vin[inIdx].prevout.serialize()
    ss += ser_string(script)
    ss += struct.pack("<q", amount)
    ss += struct.pack("<I", txTo.vin[inIdx].nSequence)
    ss += ser_uint256(hashOutputs)
    ss += struct.pack("<i", txTo.nLockTime)
    ss += struct.pack("<I", hashtype)

    return hash256(ss)
