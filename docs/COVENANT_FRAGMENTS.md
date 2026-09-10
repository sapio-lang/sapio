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
use sapio_base::fragments::{
    template_authorization_wasm_instance, templatehash_wasm_instance,
    TemplateKey,
};
use sapio_base::EmulatedProgram;

let instance = template_authorization_wasm_instance(TemplateKey::KnownTweak);
let policy = EmulatedProgram::new(instance, oracle_xpub)?;
```

Both constructors commit the exact distributed module as an inline WASM-v2
program. The v2 identity is 31 zero bytes followed by `02`. Version one keeps
the all-zero identity and its existing ABI. Applications can instead register
the module with `WasmEvaluator::with_version(WasmVersion::V2, module)` and use
the resulting versioned module hash as the evaluator ID.

Compiler output should retain the complete `EmulatedProgram`, including its
public root, for constructing signing requests. See
[program emulation](PROGRAM_EMULATION.md) for the request/response protocol.

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

`sapio_base::fragments::known_tweak_witness` encodes this evidence. The scalar
may include public BIP32 derivation tweaks and the final Taproot tweak. The
caller must account for x-only parity normalization; selecting the negated
output representative requires negating the corresponding scalar and flipping
the parity. No signing secret enters the evaluator.

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
