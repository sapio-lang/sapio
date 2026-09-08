# Developing Sapio

## Reproducible builds

Use rustup and the checked-in `rust-toolchain.toml` (Rust 1.98.1). Both workspaces
commit `Cargo.lock`; use `--locked` for builds and tests. The supported compiler
minimum is the tested pinned version. Upgrade the compiler and lockfiles in
reviewed commits rather than regenerating dependencies in CI.

Both workspaces pin the repaired `sapio-miniscript` Git source at
`04b69f69459fe3b043ca61fb649cf546d5a241b6`. Keep that revision aligned when updating
the dependency. The registry release at historical revision
`3f23950459f3424ccfeecc0bb14579ec2aec9820` does not contain these correctness
repairs. The [repair record](CTV_FORK_AUDIT.md) documents the covered behavior and
remaining limits.

Cargo reads `[patch]` only from the top-level workspace. An external application
or plugin workspace using Sapio must copy the same `[patch.crates-io]` entry from
this repository's `Cargo.toml`; the patch does not propagate through library
dependencies. See [Cargo's patch rules][cargo-patch]. Publishing supported Sapio
crates requires a repaired Miniscript release and updated dependency requirements.

A native C compiler is required for secp256k1. The WASM build additionally needs
LLVM Clang with the `wasm32` target. Apple's system Clang does not provide that
target. With Homebrew LLVM installed on macOS, set:

```sh
export CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang"
```

On Linux, install the distribution's Clang package and use
`CC_wasm32_unknown_unknown=clang`. The Rust WASM target is included in the pinned
toolchain configuration. Neither Zig nor wasm-pack is required for these builds.

## Checks

Run the native suite, feature checks, Clippy and API documentation build:

```sh
bash contrib/test.sh
```

The network integration test binds only to `127.0.0.1` on an automatically chosen
port. A sandbox must permit loopback sockets to run it. No Bitcoin node or remote
emulator is required. Clippy currently reports warnings from older code; removing
that debt is a separate milestone, not a claim that this checkout is warning-free.

Build every WASM example and exercise direct and cross-module compilation through
the CLI:

```sh
bash contrib/sapio_wasm.sh
```

This requires Python 3. The smoke test uses a temporary module cache and compares
parsed JSON with checked-in expected results. It fails on CLI errors, malformed
output, or a changed contract result. No jq or global module-cache setup is needed.
The scripts use Cargo's default target directories. The WASM script also runs
the inscription plugin's native artifact/signing tests and compiles a 521-byte
inscription through the real guest ABI. See [inscription validation](INSCRIPTIONS.md).

The pinned fork's Rust suite passes 157 tests, including 51 inscription tests.
Its dedicated node job separately checks reveals against Bitcoin Core 31.1:
five valid transactions are accepted and 21 invalid variants are rejected.
These ordinary Taproot checks run in the fork repository and do not require a
node for Sapio's commands above. The [fork's validation guide][fork-inscriptions]
provides the fixture build and isolated regtest commands.

For a faster native compiler example:

```sh
cargo run --locked -p sapio --example payment > payment.json
```

That command prints a research compilation artifact, including its explicit
backend label. It performs no funding or broadcasting.

Useful focused checks:

```sh
cargo test --locked -p sapio-psbt
cargo test --locked -p sapio-base --test ctv_hash
cargo test --locked -p sapio --test fees --test action_names --test ordinal_allocation
cargo test --locked -p sapio-wasm-plugin --features host
cargo test --locked -p sapio_integration_tests
cargo test --locked --manifest-path plugin-example/Cargo.toml -p sapio-wasm-ordinal-inscription
cargo fmt --all -- --check
cargo fmt --manifest-path plugin-example/Cargo.toml --all -- --check
```

## Book

The book builds with mdBook 0.5.4. Install it separately and render locally:

```sh
cargo install mdbook --version 0.5.4 --locked
mdbook build docs/learn-sapio
```

CI renders the book on pull requests; the existing Pages workflow publishes on
changes merged to master. A successful render does not validate every historical
code sketch. Executable tutorial coverage remains a roadmap item.

## Contract authoring

Every `Contract` declares its continuation argument type through either
`declare! {non updatable}` or `declare! {updatable<Arguments>, ...}`. There is no
nightly feature or associated-type-default variant.

Amounts in module JSON are integer satoshis. `set_min_feerate` uses satoshis per
virtual byte and checks explicitly reserved fees; it does not add fees. Repeated
minimums keep the strongest requirement. Minimum-feerate checks currently reject
additional inputs whose satisfaction weights are unknown.

WASM v1 ABI transfers are bounded to 16 MiB, including a string's terminator.
Invalid pointers, lengths, JSON and UTF-8 return errors or trap the guest call.
Plugin callbacks and metadata are registered together exactly once, without
mutable global function pointers.
Diagnostic logs go to stderr. Emulator protocol JSON frames are bounded to one
million bytes, and failed connections are discarded before another request.

The host uses Wasmer 6.1 with these fixed execution limits:

| Resource | Limit |
| --- | --- |
| Binary module source, including debug information | 128 MiB |
| Accessible linear memory per instance | One memory, at most 64 MiB |
| Table per instance | One table, at most 65,536 elements |
| Execution fuel per instance | 100,000,000 points across its entire lifetime |
| Nested module depth | Eight levels below the top-level instance |
| Child module attempts | 64 across the top-level handle and all descendants |

Fuel is installed before instantiation: WASM start functions, plugin
initialization, allocation, metadata and contract calls all consume the same
allowance. Each operator costs one point, accounted at block boundaries. Bulk
memory operations additionally cost one point per byte; memory growth costs
65,536 points per requested page. Bulk table operations and table growth cost
16 points per requested element. An exhausted allowance traps execution. Threads
are disabled so atomic waits cannot block outside the fuel accounting.
Declared memory/table maxima below the host caps remain in effect.

Nested calls reserve an attempt before loading a module, including failed
lookups. Dropping a child does not restore attempts. A new top-level handle or
`fresh_clone()` receives independent fuel, memory and nested-call allowances;
ordinary calls on an existing handle do not reset them. Allocating host callbacks
cannot recursively reenter the guest allocator. The WASM signing import accepts
only checked signature additions, just like native signing.

These are guest execution limits, not a wall-clock deadline or a process memory
limit. Native compilation, host serialization/schema work and emulator I/O are
outside instruction metering. Wasmer may reserve substantially more virtual
address space than the accessible linear-memory limit. Evaluate those costs
before exposing compilation as a service.

The cache stores binary WASM sources at `CACHE/sources/HASH.wasm` (the CLI uses
`WORKSPACE/modules/sources/HASH.wasm`), verifies their
size and content hash, and recompiles with the current host engine on every
load. It never deserializes cached native executables. Recompilation costs more
than loading a native artifact. The host skips Cranelift optimization passes
to reduce compilation latency for short-lived instances. `fresh_clone()` reuses already compiled code
while creating a separate instance. Cache corruption and I/O failures propagate
as errors.

Legacy executable caches are ignored. Reload the original `.wasm` files before
referring to their cached keys; the content hashes remain unchanged:

```sh
sapio-cli contract load --workspace PATH --file MODULE.wasm
```

## Artifact boundaries

Compiled artifacts have public fields and can be deserialized from JSON. Call
`Object::validate()` before consuming one directly. `bind_psbt` validates the
whole graph before requesting signatures or updating the transaction index;
the CLI also validates before requesting a funding transaction. Errors identify
the contract path and, when applicable, its template hash.

Binding currently supports unsigned templates whose contract input is index
zero. Template map keys and cached hashes must match the transaction, and output
amounts/scripts must match the receiving-contract metadata. Optional input
mappings contain one entry per input; entry zero must be `None` because the
graph determines the contract input. Unknown template keys are rejected.

Binding authenticates known funding transactions, checks scripts and available
amounts, and accepts only signature additions from emulators. Matching unknown
transactions remain unresolved for offline work; other lookup errors propagate.
Bound graph keys identify individual occurrences, while `source_path` retains
the original compilation path. See [the binding contract](BINDING.md) for the
funding checks, path format and limits.

Signing and finalization reject malformed PSBT maps before processing inputs.
`finalize_psbt_format_api` returns `Result<PSBTApi, PSBTValidationError>`:
structural errors are distinct from a valid PSBT still missing signatures.

The pinned Miniscript fork includes conditional scriptSig hashing and verifies
candidate and completed transactions. Whole-transaction finalization establishes
legacy scriptSigs before native witness inputs; tests cover native WSH and
Taproot CTV alongside a legacy input in either position. Explicit sighash metadata
is enforced for ECDSA and Schnorr signatures, including previously finalized
inputs. Both hash implementations cover all 400 official BIP-119 expected hashes.

Single-input finalization checks the currently known scriptSigs. Finish with
`PsbtExt::extract` or `interpreter_check` after other inputs are finalized.
Whole-transaction finalization can leave partial progress on error. Automatic
ordering does not solve circular P2SH commitments or expand bare-descriptor
support, and the builder still requires unsigned input-zero templates.

Script verification uses supplied prevouts; callers outside `bind_psbt` must
authenticate them as well. Backend enforcement and native CTV node
execution are separate release requirements. See the
[CTV fork repair record](CTV_FORK_AUDIT.md) for exact evidence and limits. The
separate Core inscription checks cover ordinary Taproot; they do not validate
native CTV, Ord indexing, sat assignment or supplied funding history.

## Contributions

Keep behavioral changes in small commits with focused regression coverage. Keep
formatting and code moves separate when possible. Preserve externally meaningful
assertions: emitted transactions, signature verification, rejection conditions,
and resource bounds. Update the roadmap when completing a release gate.

See [CONTRIBUTING](../CONTRIBUTING) for the existing contribution terms.
No license or ownership transfer policy was changed in this branch.

[cargo-patch]: https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section
[fork-inscriptions]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/docs/INSCRIPTIONS.md
