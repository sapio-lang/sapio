# Program-based covenant emulation

Sapio can compile a fixed program instance to an oracle-derived key and ask a
WASM evaluator to authorize individual transactions later. The example
below implements a predicate that CTV does not express directly: pay at least a
fixed amount to a fixed recipient, allowing the amount and output order to vary.

Run the complete example with:

```sh
cargo run --locked -p sapio_integration_tests --example program_emulation
```

It prints a serialized Sapio artifact and three locally finalized transactions.
Funding is synthetic, the oracle uses a published disposable key, and nothing is
broadcast. The executable evaluates locally; its integration test also exercises
the actual `ProgramClient`/`ProgramOracle` TCP exchange.

## Source, candidates, and authorization

The source in `integration_tests/src/program_example.rs` fixes an evaluator
identity derived from the complete compiled WASM interpreter, the
`pay-at-least/v1` program selector, recipient script, minimum
payment, and public oracle root. `ProgramInstance` commits the complete selector
and parameter bytes together with the evaluator's semantic identity.
`EmulatedProgram` derives the corresponding key from the public root and
implements `PolicyCompiler`.

The contract declares a cached `#[guard(policy)]` returning that complete
`EmulatedProgram`. Its continuation may propose a larger payment or reorder the
outputs. Those effects generate candidate transactions without changing the
guard or funded address. Changing the minimum, recipient, evaluator semantics,
program bytes, or oracle root produces different authorization.

`EmulatedProgram` remains a typed `ScriptPolicy::Program` until script lowering.
The resulting script contains its ordinary signature key, while the compiled
object retains the complete program-bearing branch in `program_policies`.
Each record includes its native guards and exact key-path or TapLeafHash
locations. The compiler records these automatically, including finish guards
and branches without suggested transactions. Optional metadata is not needed
to recover a program request.

For a nonzero evaluator ID, also preserve the matching interpreter module;
the ID authenticates those bytes but cannot reconstruct them. The example's
interpreter is distributed in `evaluators/artifacts/pay_at_least.wasm`.

Binding supplies the candidates' actual previous transactions and Taproot data.
Only after compiling and binding does the example create the evaluator and ask
it to sign the program instance, selected input, spending path, candidate PSBT,
and an auxiliary witness.

## Exact example semantics

The registered `PayAtLeast` WASM evaluator accepts only `pay-at-least/v1` as its
program selector. Parameters are an eight-byte little-endian minimum, a
four-byte little-endian recipient-script length, and exactly that many script
bytes. A witness is exactly one four-byte little-endian output index. Truncation
and trailing bytes are errors.

The predicate accepts when the selected output exists, its amount is at least
the committed minimum, and its script exactly matches the committed recipient.
The witness selects evidence for the predicate; it does not commit that witness
to Bitcoin's transaction witness stack. The output amount and script are both
part of the signature's transaction view.

The integration tests show that one funded address authorizes payments of 5,000,
6,000, and 7,000 satoshis, with the recipient in either output position. They
reject underpayment, a different recipient, a wrong or malformed output witness,
changed fixed parameters, and a different oracle root. Signatures fail
finalization after their payment amount or destination is changed.

## Bitcoin Core validation

The vector executable also checks the signature boundary against an independent
node, using two different payments from the continuation, one authenticated
program-key TapScript spend, and one inline CTV WASM spend:

```sh
cargo build --locked -p sapio_integration_tests --example program_emulation_vectors
python3 contrib/check_custom_policy.py /path/to/bitcoind /path/to/bitcoin-cli \
  target/debug/examples/program_emulation_vectors
```

The driver starts an isolated regtest node, creates its own wallet and funds the
declared scripts. It checks four valid spends and seven transactions mutated
after signing, then stops the node and removes its temporary data. CI runs the
same checks with pinned Bitcoin Core 31.1. These are ordinary Taproot spends;
the check does not establish native covenant enforcement or oracle honesty.

## Instance commitment and public derivation

`EvaluatorId` and `ProgramId` are distinct 32-byte SHA256 newtypes.
`ProgramInstance::new(evaluator, program, parameters)` validates the byte bounds;
deserialization enforces the same checks. Its fields are immutable. `id()` uses
the following exact encoding, independent of JSON whitespace or field order:

```text
tag = SHA256(UTF8("Sapio/Emulation/Program/v1"))
ProgramId = SHA256(
    tag || tag || evaluator[32] ||
    u32_le(program.length) || program ||
    u32_le(parameters.length) || parameters
)
```

The commitment encoding remains version one; the evaluator identity selects
the execution version and signed view. The all-zero identity runs an inline
WASM-v1 module, and the reserved `00..02` identity runs an inline WASM-v2 module.
Other identities commit to registered interpreter bytes and their execution
version; registration order and executable paths have no role. Both forms use
the [versioned bounded evaluator ABI](WASM_EVALUATORS.md). The interpreter must reject
invalid program and parameter encodings. Bitcoin still trusts the oracle to
run the committed implementation before using its signing key.

`program_derivation_path(id)` begins with the non-hardened index `0x53415049`
(`SAPI`). Interpret the ID as eight big-endian 32-bit words. The next eight
non-hardened children contain each word's low 31 bits; the tenth child contains
their high bits, with word zero's high bit at bit zero. The root's maximum BIP32
depth is 245. Public and private derivation use exactly this path; errors do not
substitute another path.

CTV retains its existing nine-child encoding without the prefix. These paths
are distinct under the same configured root. Arbitrary ancestor-related roots
do not establish independent namespaces or independent oracle custody.

`EmulatedProgram::new(instance, root)` checks public derivation and retains both
inputs. Its `compile_policy()` returns the typed program source. Lowering derives
the ordinary signing key and retains the source/location records. Compilation
does not register an evaluator or contact an endpoint.

## Protocol and resource limits

`emulator_connect::program` exposes `ProgramOracle`, `ProgramClient`,
`ProgramSigningRequest` and `ProgramSpendPath`. Construct an oracle with an
explicit vector of `WasmEvaluator` modules; duplicate identities are errors.
An empty vector supports inline WASM programs through the reserved v1 and v2
identities described in [the evaluator ABI](WASM_EVALUATORS.md).
Use `sign(request)` locally or `serve(prebound_listener)` for TCP. A client takes
an already resolved `SocketAddr` and public root and opens a fresh connection
for each request. It never retries or falls back to the CTV protocol.

Every wire frame has a four-byte big-endian JSON byte count followed by exactly
that many UTF-8 JSON bytes. These are the externally tagged message shapes
(`...` denotes the corresponding data, not literal wire bytes):

```text
{"SignProgramV1": {
  "instance": {"evaluator": "<64 hex digits>", "program": [...], "parameters": [...]},
  "input_index": 0,
  "witness": [...],
  "path": "KeyPath",
  "psbt": [...]
}}
{"SignedV1": [...]}
{"RejectedV1": "diagnostic reason"}
```

A script-path request uses `{"ScriptPath":"<64-digit tapleaf hash>"}` for
`path`. Byte vectors are JSON integer arrays. The PSBT wrapper is also an array:
four big-endian bytes giving the binary PSBT length, followed by its consensus
serialization. Successful responses use the same PSBT wrapper. Rejection text
is diagnostic; success requires local signature and complete-PSBT validation.
The legacy `SignPSBT` service cannot interpret `SignProgramV1` requests.

| Resource | Implemented limit |
| --- | --- |
| Exact program bytes | 65,536, in native and serialized construction |
| Exact preset parameters | 65,536, in native and serialized construction |
| Auxiliary witness | 65,536, in native signing and deserialization |
| JSON frame body | 1–1,000,000 bytes |
| Binary PSBT decoded from the wrapper | At most 1,000,000 bytes; JSON framing is also enforced |
| Admitted server connections | 64 by default; configured with `serve_with_limits` |
| Complete request I/O | 30 seconds by default; explicitly configurable |

The server deadline spans header, body and response for each request. The client
deadline also includes connection establishment. Partial progress does not
restart either deadline. These limits do not preempt synchronous evaluation,
serialization or cryptography. Evaluators themselves execute under the
[WASM fuel and memory limits](WASM_EVALUATORS.md#resource-and-execution-semantics);
native module compilation remains outside execution fuel.

The signer requires every input's previous output, accepts consistent witness
and/or authenticated non-witness UTXOs, and rejects conflicts. The selected
input must spend P2TR and have no finalized scriptSig or witness. An explicit
sighash declaration must be `ALL`; `DEFAULT`, `ANYONECANPAY`, `NONE` and `SINGLE`
are unsupported. Every produced signature uses `ALL` and either the
verified tweaked key path or an authenticated TapScript leaf without
`OP_CODESEPARATOR`. V1 signs without an annex. V2 can sign the exact annex
declared in the PSBT and exposed to the guest; see
[covenant fragments and annexes](COVENANT_FRAGMENTS.md#annexes-in-psbts).

Only the requested input/path signature slot may change. A valid existing
signature is retained after evaluation; a conflicting signature fails. The
client independently derives the key, verifies the signature and checks every
other PSBT field for exact preservation. This verifies the requested signature,
not full transaction validity or satisfaction of other spending conditions.

## Artifact-driven preparation

`Object::program_requirements()` validates the complete object graph and returns
this output's available program signature slots. Every slot contains its exact
`EmulatedProgram` and a `ProgramSpendPath`. Select a slot explicitly; multiple
programs can share a leaf and different leaves can represent alternatives.
A requirement is an available signature slot, not an instruction to sign every
program or proof that one signature satisfies the whole branch.

```rust,ignore
use emulator_connect::program::prepare_program_request;

let available = artifact.program_requirements()?;
// Select the intended program and path from `available`.
let request = prepare_program_request(
    &artifact, &selected, funded_psbt, input_index, evidence,
)?;
let signed_psbt = oracle.sign(request)?;
```

Preparation checks that the selected requirement belongs to the artifact,
authenticates available previous transactions, matches the selected input's
script and funding floor, and supplies descriptor proofs. Existing conflicting
Taproot metadata or signatures fail. Unrelated PSBT data is preserved. The
existing signing protocol then evaluates the supplied evidence and verifies
responses; finalization still checks native guards and transaction signatures.
Preparation does not choose an oracle, connect to an endpoint or evaluate WASM.

Artifact validation re-lowers each complete recorded branch, requires the
canonical live source, and checks exact leaf bytes. Unreachable alternatives
cannot advertise additional programs. A key-path record additionally needs an independently sufficient
bare program key matching the descriptor's internal key. Automatic key
selection can leave that program available through both key and script paths;
explicit key pinning removes the matching bare-key leaf. Repeated leaves and
source alternatives retain their records without mixing unrelated programs.
Source/expanded payload and node budgets bound validation and retained source.

These checks establish internal consistency, not artifact origin or an
oracle's honesty. An ordinary key cannot reveal whether someone originally
intended it to represent a program. Replacing or removing all provenance cannot
be detected cryptographically from the output script alone. Applications still
need the intended artifact from their trusted contract-building process.

The serialized `program_policies` field is required, including an empty list on
objects without programs. Recompile older artifacts. The WASM ABI and program
identity encoding are unchanged.

The integration test `program_artifacts` sends only serialized artifacts,
funded PSBTs, explicit requirements and evidence to a fresh process. It removes
optional metadata and descriptor proofs, then prepares, signs and finalizes
both template-authorization modes and eltoo update/settlement spends without
reconstructing the original Rust contract objects.

The `program-policy` WASM catalog fixture also compiles a typed program guard
inside a real guest. Host checks compare the exact program, public root and
script-path requirement after output schema validation and deserialization.

The CLI's covenant modes and `emulator_server` executable still configure the
CTV service. Generic program preparation/signing is exposed through the public
Rust API; `bind_psbt` supplies candidates without automatically dispatching
program requests. The caller chooses its evaluator registry and signer runtime.

## Relationship to SIMP

SIMP attaches optional interactive protocol data to objects, guards,
continuations and template inputs/outputs. It can describe coordination,
witness collection or presentation. Guard SIMPs retain their typed source
policy, including programs, and contextual metadata still runs at each guard
attachment independently of cached policy evaluation.

Required program source lives in `program_policies`, separately from SIMP and
`metadata.extra`. Neither the artifact validator nor program preparation treats
protocol numbers, endpoint hints or program-looking JSON as signing authority.
Removing optional metadata leaves program requirements and requests unchanged.
A contract may explicitly commit a metadata value in its spending rules; that
commitment comes from the contract, not from installing a SIMP handler.

Input SIMP insertion also preserves existing metadata on duplicate or
serialization failure. The new workflow uses explicit fallible APIs, without
a handler registry or automatic protocol dispatch.

## What the oracle guarantees

This is trusted covenant emulation. Bitcoin enforces the signature; the oracle
enforces the registered predicate. A dishonest oracle holding the signing root
can authorize a transaction that violates the predicate, and an unavailable
oracle can prevent spending. The API does not provide BitVM accountability,
fraud proofs, or penalties for violating the predicate.

`SignedTransactionView` exposes the fields covered by the supported Taproot
`SIGHASH_ALL` signature: version, lock time, input outpoints and sequences,
previous output amounts and scripts, all outputs, and the selected input index.
V2 additionally exposes the authenticated Taproot internal key and the annex
committed by the signature.
It deliberately does not expose scriptSig bytes, witness stacks, transaction
weight, txid, or arbitrary PSBT metadata as predicate inputs. An evaluator must
not base acceptance on ambient host state. The WASM host exposes only the
explicit arguments and metered public cryptographic operations. It bounds
execution but does not prove that an evaluator's predicate is correct.

These exclusions follow the [BIP341 signature
message](https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#common-signature-message).
Incorrect supplied prevout amounts or scripts cannot produce a signature valid
against the actual spent outputs. That property does not establish funding
history, confirmation or unspentness. Auxiliary witness acceptance likewise
does not publish that witness or establish a data-availability guarantee.

This implements the offline public-key derivation and conditional signing
pattern described in [Rubin's Un-FE'd Covenants](https://rubin.io/bitcoin/2024/11/26/unfed-covenants/).
The existing CTV protocol retains its derivation and request type unchanged.
The separate [CTV WASM program](WASM_EVALUATORS.md#ctv-as-a-program) implements
the template predicate through this generic protocol for native witness inputs.
