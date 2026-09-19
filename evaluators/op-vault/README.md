# OP_VAULT emulation profile

This evaluator implements a restricted version of the dynamic leaf update and
recovery operations described by historical [BIP345][bip345]. BIP345 is closed;
its proposed replacement is [BIP443][bip443]. This is an explicitly selected
oracle predicate, not a claim that either proposal is active Bitcoin consensus.

The trigger and recovery predicates execute in WASM. The oracle's signature is
the on-chain authorization: a dishonest oracle can disregard its evaluator.
Independent trigger/recovery signature guards can restrict that authority but
do not turn this emulation into a native covenant.

## Supported profile

- One selected P2TR vault input. Every other input must be a native P2WPKH or
  P2WSH fee sponsor. Additional P2TR inputs are rejected: this evaluator does
  not implement transaction-wide batching of covenant amount claims.
- A positive block delay, from 1 through 65,535, committed at vault creation.
- A BIP119 withdrawal hash chosen in the trigger witness, not at vault creation.
- Replacement of the **selected** tapscript leaf, keeping the original internal
  key and every sibling hash. The runtime authenticates the selected leaf hash;
  the guest additionally authenticates the supplied script/control-block proof.
- The replacement is `and_v(v:pk(CTV_KEY),older(DELAY))`. `CTV_KEY` is derived
  from the exact distributed inline-CTV program instance under the committed
  public oracle root. There is no native `OP_CHECKTEMPLATEVERIFY` instruction in
  this script. The existing CTV WASM evaluator checks final withdrawal.
- Optional partial revault to the same scriptPubKey as the spent vault input.
- Fixed recovery-script commitment. Trigger and recovery preserve all vault
  principal; external inputs pay their fees. Final withdrawal may allocate its
  committed amount to fees.

The profile requires ordinary tapscript version `0xc0`, source scripts of at
most 10,000 bytes, and control blocks with at most 128 siblings. The trigger and
revault outputs must be distinct. Revault cannot exceed the selected principal.
It uses one fixed replacement script shape, not arbitrary BIP345 script bodies.
Recovery relay policies, watchtower operation, and automatic fee selection are
not implemented by the predicate.

## Exact bytes

The evaluator uses `ProgramInstance::wasm_v2`. Program input is empty; immutable
parameters choose the operation. All multi-byte integers below are little
endian; hashes and keys retain their byte encoding.

| Operation | Parameters | Emulator witness |
| --- | --- | --- |
| Trigger | `00`, delay `u16`, BIP32 public root `78 bytes` | withdrawal hash `32 bytes`, trigger output `u32`, revault output `u32`, revault amount `u64`, source-script length `u32`, source-script bytes, control-block length `u32`, control-block bytes |
| Recovery | `01`, recovery commitment `32 bytes` | recovery output `u32` |

Absent revault is exactly output `0xffffffff` with amount zero. Present revault
uses a nonnegative output index and a positive amount. Trailing bytes are
rejected. These are emulator evidence codecs, not Bitcoin Script stack-number
encodings. Public constructors in `sapio_base::op_vault` create the same codecs.

The recovery commitment is tagged SHA256 with tag `VaultRecoverySPK` over
`CompactSize(script length) || script`. Amounts associated with the chosen
outputs must cover their allocated portions of the selected input; overfunding
is permitted. Each operation reads actual signed input/output amounts, so
contract compilation does not establish the later funding amount by itself.

## Context and native imports

The new `sapio_context_v2.tapleaf_hash(output_pointer)` import writes the
authenticated selected tapleaf hash and returns `1` for script-path signing.
It returns `0` for key-path signing without writing output. The trigger rejects
key-path invocation. This additive import leaves existing v2 view bytes intact.

The guest uses the existing metered SHA256, BIP32 public derivation, and x-only
tweak-check imports. Rebuilding different guest or embedded CTV bytes changes
the predicate identity and derived signature keys. `evaluators/build.sh`
checks/writes the CTV artifact before compiling this dependent evaluator.

[bip345]: https://github.com/bitcoin/bips/blob/master/bip-0345.mediawiki
[bip443]: https://github.com/bitcoin/bips/blob/master/bip-0443.mediawiki
