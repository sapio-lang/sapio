# Developing Sapio

## Reproducible builds

Use rustup and the checked-in `rust-toolchain.toml` (Rust 1.98.1). Both workspaces
commit `Cargo.lock`; use `--locked` for builds and tests. The supported compiler
minimum is the tested pinned version. Upgrade the compiler and lockfiles in
reviewed commits rather than regenerating dependencies in CI.

Both workspaces pin the repaired `sapio-miniscript` Git source at
`f62ebf16db55732f9efc8d7d5292979332274cad`. Keep that revision aligned when updating
the dependency;
the registry release at historical revision
`3f23950459f3424ccfeecc0bb14579ec2aec9820` does not contain these CTV finalization
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
The scripts use Cargo's default target directories.

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

These checks do not establish a complete sandbox: execution metering, aggregate
memory limits, nested-call limits, and compiled-cache trust still need work.
Use modules and module caches you trust. The runtime uses native compiled
artifacts internally; a hash-shaped filename is not a trust boundary.

The host uses Wasmer 6.1 for compatibility with current Rust on x86 Linux.
Compiled module caches are runtime-specific. After upgrading from Wasmer 4,
reload the original `.wasm` files before referring to their cached keys:

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

Script verification uses supplied prevouts; funding UTXO authentication remains
required at the caller boundary. Backend enforcement and native CTV node
execution are separate release requirements. See the
[CTV fork repair record](CTV_FORK_AUDIT.md) for exact evidence and limits; library
tests do not establish chain enforcement or general CTV transaction support.

## Contributions

Keep behavioral changes in small commits with focused regression coverage. Keep
formatting and code moves separate when possible. Preserve externally meaningful
assertions: emitted transactions, signature verification, rejection conditions,
and resource bounds. Update the roadmap when completing a release gate.

See [CONTRIBUTING](../CONTRIBUTING) for the existing contribution terms.
No license or ownership transfer policy was changed in this branch.

[cargo-patch]: https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section
