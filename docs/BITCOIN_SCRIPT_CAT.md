# Bitcoin Script with CAT under the program oracle

`sapio_base::bitcoin_script::emulated_script(script, oracle_root)` commits the
exact executable Script bytes and distributed WASM interpreter to a program
key. [Purrfect Vault](PURRFECT_VAULT.md) supplies its covenant as ordinary Script
parameters and initial-stack evidence. The guest does not recognize stages.

## Supported execution profile

The interpreter supports pushes, small integers, DUP/2DUP, DROP, SWAP, ROT,
TOALTSTACK/FROMALTSTACK, CAT, SIZE, VERIFY, EQUAL/EQUALVERIFY, LESSTHAN/GREATERTHAN,
SHA256/HASH256, CHECKSIG/CHECKSIGVERIFY and CHECKSEQUENCEVERIFY. Other opcodes,
including remaining OP_SUCCESS bytes, fail closed. This is a bounded Tapscript
subset, not a complete Bitcoin consensus interpreter.

The selected native Script-path input is index zero of exactly two inputs.
Its companion must be native witness-v0, excluding a second Taproot covenant
whose principal could otherwise be counted twice. Annexes are excluded. These
are generic interpreter-profile checks, independent of the executed contract.

CHECKSIG accepts 32-byte keys and DEFAULT or ALL signatures. Empty signatures
are false; malformed or invalid nonempty signatures terminate evaluation.
Unknown key types and other sighash modes are rejected. Pushes and numeric
operands use minimal encodings. CSV applies BIP112’s version, disable flag,
unit and sequence-mask rules; Bitcoin still checks actual maturity.

| Resource | Bound |
| --- | --- |
| Script | 10,000 bytes |
| Initial evidence | 65,536 bytes |
| Main and alternate stacks combined | 1,000 elements |
| Pushes and CAT results | 520 bytes each |
| Ordinary numeric operands / CSV operand | Four / five bytes |
| Guest memory | Four MiB, including a fixed stack arena |

Success requires exactly one true main-stack item, including Bitcoin’s
negative-zero rules. Altstack contents do not count toward cleanstack. Each
nonempty signature operation consumes 50 units from a BIP342-style budget:
50 plus the virtual witness’s consensus serialization, Script and a minimum
33-byte control block. WASM instructions and crypto imports also share the
oracle’s fixed fuel allowance.

## Evidence and CHECKSIG context

`encode_stack(&items)` writes a little-endian u32 count, then each item’s u32
byte length and bytes. The guest checks bounds, truncation and trailing data
independently. This is off-chain evidence, not an automatically published
transaction witness.

CHECKSIG reconstructs BIP341 from the signed transaction projection and uses
**the authenticated physical wrapper leaf hash**, obtained from the runtime.
Evidence cannot select another leaf. The hash of a hypothetical native CAT
leaf would be a different signing context. CODESEPARATOR is unsupported.

The oracle issues its ordinary wrapper signature only after interpretation
succeeds. Native wrapper policies may add keys and timelocks, as the vault
does. The oracle’s honesty and availability assumptions remain; this does not
establish CAT consensus support on the target chain.
