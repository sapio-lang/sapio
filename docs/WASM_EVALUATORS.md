# WASM evaluator version one

Every program evaluation runs in a fresh, bounded WASM instance. The compiler
still derives public program keys without contacting an oracle. At signing time,
the oracle validates the PSBT and selected signature context, evaluates the
committed program, and signs only after an explicit acceptance.

## Inline programs and registered interpreters

`EvaluatorId::default()` is the reserved all-zero identity. With this identity,
`ProgramInstance::program()` contains the complete WASM module. Construct an
inline instance with `ProgramInstance::wasm(module_bytes, parameters)`.

A nonzero identity identifies a registered WASM interpreter. Its identity is:

```text
tag = SHA256(UTF8("Sapio/Emulation/Evaluator/Wasm/v1"))
EvaluatorId = SHA256(tag || tag || exact_module_bytes)
```

`EvaluatorId::for_wasm(bytes)` computes this identity during public compilation.
The oracle registers the same bytes using `WasmEvaluator`. The instance's
program bytes then become input to that interpreter. This lets another policy
language implement its evaluator in WASM and define its own program encoding.
There are no registered native Rust evaluator callbacks.

Both forms use the same version-one runtime and ABI. The zero identity cannot
be overridden. Duplicate registered identities are errors. Different module
bytes, including custom sections, identify different interpreters or inline
programs even if their observable behavior happens to agree. Preserve the
exact artifacts; recompilation is not an identity-preserving operation.

## Guest ABI

An evaluator exports one linear memory as `memory`, and these functions:

```text
sapio_alloc_v1(length: u32) -> u32
sapio_evaluate_v1(
    program_pointer: u32, program_length: u32,
    parameters_pointer: u32, parameters_length: u32,
    view_pointer: u32, view_length: u32,
    witness_pointer: u32, witness_length: u32
) -> i32
```

Pointers are offsets into that memory. The allocator returns disjoint ranges
large enough for the requested input. The host validates ranges and writes the
arguments before calling the evaluator. Empty arguments have length zero and
must not be dereferenced. Inline modules receive an empty `program` argument;
registered interpreters receive the instance's exact program bytes.

Return `1` to accept and `0` to reject. Any other return value, invalid ABI,
unsupported import, trap, or exhausted fuel is an evaluation error and produces
no signature. Evaluator modules with a start section are rejected. Mutable
memory and globals never carry over between requests.
The host exposes no WASI, network, filesystem, clock, randomness, module lookup,
compiler callbacks, or signing secrets to evaluators.

## Signed transaction encoding

All integer fields use little endian. The view has this exact layout:

| Field | Encoding |
| --- | --- |
| Transaction version | i32 |
| Lock time | u32 |
| Selected input index | u32 |
| Input count | u32 |
| Each input | 36-byte consensus outpoint, u32 sequence, u64 previous value, u32 previous script length, script bytes |
| Output count | u32 |
| Each output | u64 value, u32 script length, script bytes |

Outpoints contain the 32 transaction-hash bytes in consensus serialization
order followed by the u32 output index. Script lengths and counts in this ABI
are fixed-width integers, not Bitcoin CompactSize encodings. The view contains
no scriptSigs, witness stacks, txid, weight, finalization state, or arbitrary
PSBT metadata. Every exposed transaction field is covered by the supported
Taproot `SIGHASH_ALL` signature. See [BIP341's signature
message](https://bips.dev/341/#common-signature-message).

The auxiliary witness is separately supplied evidence. Acceptance does not
publish it on chain or establish a data-availability guarantee.

## Native cryptography

Compiler plugins and evaluators share the `sapio_crypto_v1` import namespace.
The `sapio-wasm` crate provides `no_std` guest wrappers. Operations receive
explicit public byte strings and write fixed-size results into guest buffers.

| Import | Arguments | Result | Fuel |
| --- | --- | --- | --- |
| `sha256` | input pointer, byte length, 32-byte output pointer | 0 on success | 100 + 2 per input byte |
| `bip32_derive` | 78-byte encoded xpub pointer, u32-LE path pointer, child count, 33-byte compressed key output pointer | 0 on success; 1 for invalid derivation data | 500 + 50,000 per child |
| `schnorr_verify` | 32-byte message pointer, 32-byte x-only key pointer, 64-byte signature pointer | 1 valid; 0 invalid | 50,000 |

SHA256 accepts at most 1 MiB. BIP32 paths contain at most 255 non-hardened
children and must fit the root's remaining depth. Invalid memory ranges trap.
Input/output overlap is supported by reading inputs before writing results.
No operation derives a secret key or produces a signature.

The host deducts the charge from the same nonrenewable fuel counter used by
WASM instructions, before copying inputs or performing cryptography. Failed
operations spend their charge too. Host calls cannot refill fuel or clear an
exhaustion flag. Crypto calls during module initialization before memory and
metering handles are bound fail closed.

Sapio's CTV and generic program public-key derivation use this host path when
compiled to WASM. Other Bitcoin/Miniscript operations may still use guest-side
cryptography; this does not remove all secp256k1 code from compiler modules.

## Resource and execution semantics

Version one uses 100,000,000 fuel points per instance, shared by initialization,
allocation, evaluation and host calls. Ordinary instructions cost one point.
Bulk memory operations additionally cost one point per byte; memory growth
costs 65,536 per page; table bulk operations cost 16 per element. Memory is
bounded to 64 MiB and tables to 65,536 elements, preserving any smaller declared
maximum. Threads and shared memory are unavailable. Evaluator SIMD is disabled,
and floating-point arithmetic NaNs are canonicalized.
An unsuccessful evaluator `memory.grow` or `table.grow` traps before the guest
can observe `-1`, including when the requested size exceeds its declared limit.
This prevents a predicate from accepting because host allocation failed.
Successful growth still returns the previous size. Compiler plugins retain
ordinary WASM growth-result behavior.

Program and interpreter modules remain limited to 65,536 bytes. Parameters,
program input and auxiliary evidence each retain their 65,536-byte bounds.
The encoded signed view is bounded to 1 MiB before copying it into guest memory.
The existing transport frame limit also applies; these bounds are not additive
permission to exceed that frame.
Before native compilation, a binary preflight limits the sum of parameters
and declared locals across defined functions to 65,536 slots. Shared function
types count separately for each function that uses them. This bounds compact
local declarations that could otherwise expand a small module into substantial
compiler work. It is separate from execution fuel.

Native compilation and operating-system allocation are outside execution fuel.
A server I/O deadline cannot interrupt synchronous compilation. Module-size
bounds limit admitted source, and a runtime/resource failure is an error rather
than predicate acceptance. The runtime is not a proof system or a guarantee
that every host can execute every admitted program successfully.

Incompatible changes to the evaluator ABI, host operations, view encoding or
cost schedule need a new evaluator version. Do not silently redefine the zero
identity to mean the latest runtime configuration.

## CTV as a program

`sapio_base::program::ctv_wasm_instance(Ctv(expected_hash))` constructs an inline
instance using the checked-in `CTV_WASM` artifact. The Rust guest in
`evaluators/ctv` imports only generic SHA256. Parameters are exactly 32 raw hash
bytes, and the program and witness arguments must be empty.

The guest requires **every input** to spend a native witness program. For those
spends, consensus requires an empty scriptSig. Legacy and P2SH inputs, including
wrapped SegWit, are rejected. An empty unsigned PSBT scriptSig alone would not
establish this invariant: Taproot signatures do not commit other inputs'
scriptSigs. See [BIP141's native witness
rule](https://bips.dev/141/#witness-program).

For this domain, the guest computes the exact 84-byte [BIP119
preimage](https://bips.dev/119/#detailed-specification):

```text
version_i32LE || locktime_u32LE || input_count_u32LE ||
SHA256(all sequences_u32LE) || output_count_u32LE ||
SHA256(all consensus-serialized TxOuts) || selected_input_index_u32LE
```

It hashes that preimage once and compares the result to its parameters. Output
scripts use Bitcoin CompactSize lengths during this hash computation, even
though the evaluator ABI uses u32 lengths. No special CTV host operation or
native CTV evaluator participates in the decision.

The generic program key commits to these WASM bytes and parameters. It differs
from the older nine-child CTV emulation key. Existing `LoweringPlan::CtvEmulation`
artifacts and the CTV transport retain their explicit meaning; they are not
silently reinterpreted as this program. Use `EmulatedProgram` and `ProgramOracle`
for the WASM path and preserve the complete program metadata.

## Building and verifying the distributed programs

`evaluators/` contains dependency-free Rust sources for CTV and the registered
flexible-payment evaluator. Its pinned toolchain and build settings produce
small `no_std` artifacts with names and debug sections stripped. Verify exact
bytes with:

```sh
bash evaluators/build.sh --check
```

After an intentional source or toolchain change, `--write` replaces the
artifacts. Review the binary change together with the source; it changes the
corresponding program identity and funded address. Normal library builds use
the checked-in artifacts and do not require compiling a guest toolchain.
