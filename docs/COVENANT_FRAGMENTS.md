# Covenant fragments in WASM

The `sapio-covenant-fragments` guest library in `evaluators/fragments` composes
TemplateHash, CSFS, authenticated internal-key lookup, and known-tweak
authorization inside one bounded WASM evaluator. Each fragment produces a typed
value or predicate result. No separate oracle round trip is needed between
fragments.

The distributed `templatehash.wasm` guest compares the current BIP446 hash to
32 fixed parameter bytes. `template_authorization.wasm` computes that hash and
checks a witness signature under a pinned key, the physical Taproot internal
key, or a key authenticated by a known additive tweak.

## Public construction

```rust,ignore
use sapio_base::fragments::{template_hash_eq, template_signed_by, TemplateKey};

let authorized = template_signed_by(TemplateKey::InternalKey, oracle_xpub)?;
let settlement = template_hash_eq(expected_hash, oracle_xpub)?;
```

Both helpers return `EmulatedProgram`, ready for a `#[guard(policy)]` method
or composition with native guards through `ScriptPolicy::And`. The oracle
root is an explicit public input. They commit the exact distributed module as an inline WASM-v2
program. The v2 identity is 31 zero bytes followed by `02`. Version one keeps
the all-zero identity and its existing ABI. Applications can instead register
the module with `WasmEvaluator::with_version(WasmVersion::V2, module)` and use
the resulting versioned module hash as the evaluator ID.

Compiler output automatically retains the complete `EmulatedProgram`, its
public root and guarded spending locations in `program_policies`. Use
`artifact.program_requirements()` and `prepare_program_request` to construct
an explicit signing request. See [program emulation](PROGRAM_EMULATION.md#artifact-driven-preparation)
for artifact validation and the request/response protocol.

## TemplateHash and CSFS

TemplateHash implements [BIP446](https://github.com/bitcoin/bips/blob/master/bip-0446.md):

```text
TaggedHash("TemplateHash",
    version_i32LE || locktime_u32LE || SHA256(sequences_u32LE) ||
    SHA256(consensus_serialized_outputs) || annex_present_u8 ||
    input_index_u32LE || [SHA256(CompactSize(annex_length) || annex)])
```

The message is 77 bytes without an annex or 109 with one, before tagged-hash
prefixes. There are no explicit input/output counts or scriptSig commitments.
Companion inputs may spend legacy outputs. Input outpoints and previous amounts
may change without changing this template hash, while the oracle's final
transaction signature still commits them. The native
`sapio_base::fragments::template_hash` helper computes authorization messages
for callers; the oracle evaluates the actual compiled WASM guest.

The guest's `check_sig_from_stack(message, key, signature)` uses raw BIP340
messages without an implicit prehash. It implements the 32-byte public-key
fragment of [BIP348](https://github.com/bitcoin/bips/blob/master/bip-0348.md):
an empty signature yields `Ok(false)`, a valid 64-byte signature yields
`Ok(true)`, and every invalid nonempty signature yields a terminal `Failure`.
There is no Bitcoin sighash byte in a CSFS signature. Unknown public-key types
and a full Script stack interpreter are outside this typed fragment API.

Propagate failures when composing predicates. Ordinary Rust `&&`, `||`, and
`if` introduce conditional evaluation; they are not an eager Script `BOOLAND`
or `BOOLOR`. Converting a failed nonempty signature into `false` would let a
negated predicate incorrectly accept it.

## Explicit internal keys

Contracts may implement:

```rust,ignore
fn pinned_internal_key(&self, ctx: &Context)
    -> Result<Option<XOnlyPublicKey>, CompilationError>
```

`Some(key)` fixes the output's internal key exactly. The compiler requires an
existing independently sufficient bare-key spending branch for that key;
pinning a key protected by a timelock, hashlock, or threshold is an error.
Authors can explicitly authorize a key path through an ordinary bare finish
guard. Adding another eligible key cannot override a pin. `None` retains the
deterministic default selection.

The runtime exposes the physical internal key in its v2 context. For a
script-path signature it comes from a control block verified against the
selected previous output. For a key-path signature the runtime verifies the
program key's Taproot tweak against that output. Optional PSBT internal-key
and Merkle-root metadata must agree with the verified context.

This implements the value described by
[BIP349](https://github.com/bitcoin/bips/blob/master/bip-0349.md). It also makes
that authenticated value available during key-path emulation, where no
on-chain script executes IKEY.

## Known-tweak authorization

When the physical output key is an emulator-derived key, a witness can
authenticate an untweaked signing key without revealing a control block:

```text
P[32] || t[32 big-endian] || output_parity[1] || signature[0 or 64]
```

The `KnownTweak` fragment checks `Q = lift_x(P) + t*G` against the selected
prevout's x-only output key and the supplied full-point parity. It requires a
canonical scalar below the curve order and parity `0` or `1`. Zero tweaks are
valid; points at infinity and malformed keys fail. CSFS then verifies the
signature under P over the transaction-derived template hash. Neither the
anchor Q nor the message is selected by the witness.

For a program derived from the authorizer's public BIP32 root, use the public
proof helper. It includes BIP32 derivation, the final Taproot tweak and both
x-only parity normalizations:

```rust,ignore
use sapio_base::fragments::KnownTweakProof;

let proof = KnownTweakProof::for_program(&program, input.tap_merkle_root)?;
proof.check_output(actual_output_key)?;
let witness = proof.witness(&template_signature);
```

The runner obtains `actual_output_key` from the spent P2TR output and signs
TemplateHash with the untweaked root key. No signing secret enters the proof
constructor or evaluator. `known_tweak_witness` remains available for callers
constructing other additive openings; those callers must account for their
own scalar and parity relationship.

This fragment authenticates a key with a known additive relationship to Q.
It does not assert that P is the physical BIP341 internal key or that t equals
one particular derivation. Those are separate statements. Supplying an
arbitrary related point does not authorize a spend without its valid CSFS
signature.

## Annexes in PSBTs

[BIP371](https://github.com/bitcoin/bips/blob/master/bip-0371.mediawiki) already
provides control blocks, internal keys and Merkle roots. It has no dedicated
annex field. Sapio uses one input proprietary field:

| Component | Encoding |
| --- | --- |
| Identifier | UTF-8 `sapio` |
| Subtype | `1` |
| Key data | UTF-8 `annex` |
| Value | Exact annex bytes, including `0x50` |

Use `sapio_psbt::annex::set(input, Some(bytes))` before signing, or `None` for
absence. A present value must begin with `0x50` and contain at most 65,536
bytes. Empty is invalid. The same bytes enter the guest context, BIP446 hash,
and BIP341 signature hash. Version-one evaluators reject annex requests.

Use `sapio_psbt::finalize::finalize` or the existing CLI finalization endpoint.
Finalization verifies the actual signatures and emits the annex as the last
witness element. The original Miniscript finalizer does not support annexes.
Changing, adding, or dropping the annex after signing invalidates the signature.

Auxiliary CSFS signatures and tweak proofs are off-chain evidence. They are
not automatically placed in the annex. Applications that need publication
must deliberately commit and publish the relevant data.

## Contract source and execution

The public [template authorization contract](../sapio-contrib/src/contracts/template_authorization.rs)
contains the destinations, fee, authorization policy and payment continuation.
Its constructor takes public keys and addresses; its continuation uses the
funding amount in the caller's `Context`. With no payment proposal it compiles
only the spending policy. Proposed amounts never change that policy.

The [example runner](../integration_tests/src/fragment_example.rs) supplies
fixtures, compilation effects and signatures. It selects an explicit recorded
requirement from the compiled artifact and passes it to
`prepare_program_request`. The shared API checks funding and supplies the
compiler's descriptor proofs, rejecting conflicting PSBT data. No ad hoc
program metadata or original contract object is needed to prepare a spend.

`ProgramSpendPath::script_for` remains a lower-level selector for callers with
an explicit public program and Miniscript PSBT leaves. Artifact-based callers
use the compiler's recorded locations, including policies containing raw
fragments. Key-path selection remains explicit.

## Execution and verification

V2 uses the existing WASM fuel, memory, module-size and input limits. It adds
bounded native raw-message Schnorr verification and x-only tweak checking;
all imports share the same nonrenewable instruction fuel counter. See
[the versioned ABI](WASM_EVALUATORS.md#version-two).

The test suite runs the actual distributed guests, checks official template
hash/signature vectors, mutates authenticated transaction fields and witness
components, and verifies both key-path and script-path signing. It also checks
template authorization reuse across different outpoints, which requires a
fresh transaction signature even when the auxiliary authorization is reused.

Run the compiled contract demo and its integration test from the repository
root:

```sh
cargo run --locked -p sapio_integration_tests --example covenant_fragments
cargo test --locked -p sapio_integration_tests --test covenant_fragments
```

The demo emits four finalized synthetic spends: two payment candidates using
an explicitly authorized participant internal key, and two using a public
BIP32 root's known tweak. Candidate amounts change without changing either
contract's address. The root authorizer signs the auxiliary TemplateHash
without deriving the oracle's private signing key. No transactions are
broadcast.

To check real funded spends against an isolated Bitcoin Core regtest node:

```sh
cargo build --locked -p sapio_integration_tests --example covenant_fragment_vectors
python3 contrib/check_custom_policy.py --accept-nonstandard \
    /path/to/bitcoind /path/to/bitcoin-cli \
    target/debug/examples/covenant_fragment_vectors
```

The driver checks two valid spends and six mutations covering changed outputs,
changed annex bytes, and removed annexes. The annex-bearing valid spends need
the explicit `--accept-nonstandard` relay-policy opt-in on the isolated test
node; consensus signature validation remains enabled. The driver creates its
own temporary node and wallet and does not use existing funds.

The oracle remains responsible for enforcing the evaluated predicate under
the existing [emulation assumptions](PROGRAM_EMULATION.md#what-the-oracle-guarantees).

The [eltoo-style channel example](ELTOO_FRAGMENTS.md) composes these same v2
guests with native state-ordering and contest-delay guards. It demonstrates
reusing one update authorization against stale states, reconstructing old
Taproot proofs from published data, and settling the latest balances.
