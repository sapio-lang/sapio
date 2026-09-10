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

The resulting script contains an ordinary signature key. `PaymentContract`
therefore explicitly retains the complete `EmulatedProgram` in its
`emulated_program` object metadata. It is not recorded in the CTV-specific
`covenant_requirements`, and `bind_psbt` does not infer or dispatch a generic
program request from a key. An application must preserve this source and make
its intended request explicitly.
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
inputs. Its `compile_policy()` returns the resulting ordinary key clause.
Compilation does not register an evaluator or contact an endpoint.

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

## Artifact and CLI boundary

The complete program source must survive lowering, as it does in this example's
metadata. A key alone cannot recover its evaluator or parameters. Ordinary
`Object::validate`, CTV `covenant_requirements`, `validate_for_emulator` and
`bind_psbt` do not infer a generic signing request or verify arbitrary program
metadata against the spending script. Artifact provenance remains required.

The CLI's covenant modes and `emulator_server` executable configure the CTV
service. They do not register program evaluators or dispatch generic requests.
Applications currently use the public Rust API and preserve their intended
`EmulatedProgram` explicitly, as the executable example does. There is no
new signer import in the WASM ABI or ambient compiler callback.

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
