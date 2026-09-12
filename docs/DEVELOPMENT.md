# Developing Sapio

## Reproducible builds

Use rustup and the checked-in `rust-toolchain.toml` (Rust 1.98.1). All workspaces
commit `Cargo.lock`; use `--locked` for builds and tests. The supported compiler
minimum is the tested pinned version. Upgrade the compiler and lockfiles in
reviewed commits rather than regenerating dependencies in CI.

The native and compiler-plugin workspaces use Bitcoin 0.32.102, its upstream
secp256k1 0.29.1 dependency, and Miniscript 13.1.0. The Git revisions are pinned
in both workspace manifests and lockfiles. Bitcoin carries two narrow parser
repairs: canonical Taproot signature encodings and complete PSBT slice decoding.
Miniscript retains Sapio's CTV/inscription extensions and transaction-wide PSBT
finalization checks. See the [migration guide](BITCOIN_032.md) for API changes
and the [historical repair record](CTV_FORK_AUDIT.md) for their origins.

Cargo reads `[patch]` only from the top-level workspace. An external application
or plugin workspace using Sapio must copy the complete `[patch.crates-io]` table
from this repository's `Cargo.toml`; patches do not propagate through library
dependencies. The `bitcoin_hashes` entry keeps Bitcoin and secp256k1 on one
identical hash implementation. See [Cargo's patch rules][cargo-patch]. Publishing
supported Sapio crates requires repaired dependency releases and updated
requirements.

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

The network tests bind only to `127.0.0.1` on automatically chosen ports.
A sandbox must permit loopback sockets to run them. No Bitcoin node or remote
emulator is required. Clippy currently reports warnings from older code; removing
that debt is a separate milestone, not a claim that this checkout is warning-free.

Test every WASM example natively, build optimized release guests, and run the complete
[example catalog](EXAMPLES.md) through the real host:

```sh
bash contrib/sapio_wasm.sh
```

This requires Python 3. The catalog checks that every workspace guest has a
fixture, compiles all 20 modules with their actual schemas, validates complete
artifacts, checks payment/timing and program requirements, compares fresh-instance
results, and rejects malformed arguments. The two shared interface crates are inventoried
separately. A CLI smoke test additionally covers cross-module calls, a 521-byte
inscription, and mock binding. Each catalog process has a 360-second wall-clock allowance for its two create
requests; each individual CLI request has 180 seconds. Both allowances include
native compilation of nested modules.
Checks use temporary caches and report each case's duration. Guest release
optimization is required for the catalog: unoptimized compiler guests can
approach the 128 MiB module size limit and exhaust fuel on small contracts. No jq or global
module-cache setup is needed. See [inscription validation](INSCRIPTIONS.md).

The scripts use Cargo's default target directories. The native suite also tests
the mining payout compiler and the payment example. For custom build directories,
run the same commands with `CARGO_TARGET_DIR` and pass the resulting binary and
WASM paths to `contrib/check_examples.py` and `contrib/check_wasm.py`.

The pinned Miniscript fork passes 227 library/integration tests, including 51
inscription tests, and four differential-fuzzer regressions. Bitcoin's library
suite passes 429 tests.
Its dedicated node job separately checks reveals against Bitcoin Core 31.1:
five valid transactions are accepted and 21 invalid variants are rejected.
These ordinary Taproot checks run in the fork repository and do not require a
node for Sapio's commands above. The [fork's validation guide][fork-inscriptions]
provides the fixture build and isolated regtest commands.
Sapio also has [custom-policy node checks](POLICY_BACKENDS.md#node-validation):
two valid composed Taproot spends and ten invalid variants, with their own
fixture exporter and isolated Core driver. The Ubuntu native CI job runs them.

For a faster native compiler example:

```sh
cargo run --locked -p sapio --example payment > payment.json
```

That command prints a research compilation artifact, including its explicit
backend label. It performs no funding or broadcasting. The [native examples
guide](../examples/README.md) also describes the runnable mining payout tree.

For a continuation whose fixed policy admits several different payments:

```sh
cargo run --locked -p sapio_integration_tests --example program_emulation
```

The [program-emulation example](PROGRAM_EMULATION.md) prints its complete
program source metadata, compiled artifact and three locally finalized spends.
It uses synthetic funding and a published disposable oracle key; it performs
no wallet activity or broadcasting. Its tests also exercise the TCP protocol.

For an eltoo-style state update and delayed settlement using the existing v2
TemplateHash/IKEY/CSFS fragments:

```sh
cargo run --locked -p sapio_integration_tests --example eltoo
cargo test --locked -p sapio_integration_tests --test eltoo_security
```

The [eltoo guide](ELTOO_FRAGMENTS.md) explains reusable joint authorization,
old-state proof recovery and external fee inputs. Its isolated Core scenario
confirms a stale update, supersedes it and checks the latest state's full
contest delay before settlement. The Ubuntu CI job runs that scenario.

The dependency-free `evaluators/` workspace contains the compiled CTV,
flexible-payment, TemplateHash and template-authorization WASM programs, plus
the reusable typed [covenant fragment SDK](COVENANT_FRAGMENTS.md). Normal builds
use their checked-in artifacts.
Run `bash evaluators/build.sh --check` to rebuild with pinned settings and
verify exact bytes, or `--write` after an intentional source change. Changed
bytes change program identities and derived keys. The [WASM evaluator
guide](WASM_EVALUATORS.md) specifies the ABI, crypto costs and CTV input domain.

Useful focused checks:

```sh
cargo test --locked -p sapio-psbt
cargo test --locked -p sapio-base --test ctv_hash
cargo test --locked -p sapio-base --test program
cargo test --locked -p sapio --test fees --test action_names --test ordinal_allocation
cargo test --locked -p sapio --test conditional_compilation --test guard_semantics --test macro_declarations
cargo test --locked -p sapio --test template_semantics --test effect_names --test inscription_guards
cargo test --locked -p sapio --test custom_policy --test raw_artifacts --test contract_policy_limits
cargo test --locked -p sapio --test child_funding --test template_size --test ordinal_allocation
cargo test --locked -p sapio-wasm-plugin --features host
cargo test --locked -p sapio-wasm --features host
cargo test --locked -p sapio_integration_tests
cargo test --locked -p ctv_emulators --lib
cargo test --locked -p ctv_emulators --lib program::
cargo test --locked -p sapio_integration_tests --test program_emulation
cargo test --locked --manifest-path plugin-example/Cargo.toml --workspace
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

Context funding, template amounts, and `AmountU64` fields use integer satoshis.
Some example inputs use `AmountF64` or explicit `as_btc` serialization for BTC
amounts; consult the module schema and its checked-in fixture rather than infer
units from a JSON number. All contract accounting uses integer `Amount` values.
`Builder::add_amount` is fallible
and requires an auxiliary input (`add_sequence`) before declaring external
funding. With tracked ordinals, allocate the entire original input before adding
unknown external sats. The ordinal planner preserves input range order, places
each requested ordinal at its output's first sat, and puts fees last. Its greedy
payout placement can reject a layout that a more expensive packing search could
solve; it never reorders the input ranges to make it fit.

Timelock JSON retains Bitcoin's encoded values: relative time is the type bit
`1 << 22` plus 512-second units, not a count of seconds. Deserialization rejects
wrong type bits and out-of-range values. Converting a `Duration` rounds up to the
next representable unit, so the lock never matures earlier than requested.

`set_min_feerate` uses satoshis per
virtual byte and checks explicitly reserved fees; it does not add fees. Repeated
minimums keep the strongest requirement. Minimum-feerate checks currently reject
additional inputs whose satisfaction weights are unknown.

WASM v1 ABI transfers are bounded to 16 MiB, including a string's terminator.
Invalid pointers, lengths, JSON and UTF-8 return errors or trap the guest call.
Plugin callbacks and metadata are registered together exactly once, without
mutable global function pointers.
Diagnostic logs go to stderr. Emulator protocol JSON frames are bounded to one
million bytes; [emulator service limits](#emulator-service-limits) cover request
I/O and admitted connections.

The workspace uses Schemars 1.2.2 from the
[`sapio-jsonschema`](https://github.com/sapio-lang/sapio-jsonschema) fork. Its optional
`bitcoin032` and `miniscript13` features describe the dependencies' native JSON
representations. The fork retains the `schemars` and `schemars_derive` package
names so the root and plugin workspace patches select one shared schema trait.
Plugin and continuation interfaces explicitly generate Draft 7 schemas to match
the host validator. Plugin input schemas describe deserialization; output schemas
describe serialization, including asymmetric Serde names and skipped fields.
Denomination adapters still carry their matching schema annotations; ordinary
keys, addresses, scripts, policies, and transactions do not need local schema
substitutes.

The native host validates every actual call against the module's advertised
JSON Schemas, including calls made through raw nested-module imports. It checks
the complete `CreateArgs` input before invoking the create function, then checks
each successful output before returning it. A module's `Err(String)` remains an
ordinary module error. Schema acceptance does not prove contract behavior or
compatibility for every value of another interface.
Validation reads the advertised JSON directly, preserving integer limits above
the exact range of floating point numbers.

Plugin API generation explicitly selects Draft 7. References must resolve
within the advertised schema: validation never fetches network resources or
local files.
Patterns use the Rust regex engine, so backreferences and lookaround are not
supported. A preflight rejects nonproductive reference cycles and excessive
reference expansion (65,536 schema visits or 128 levels), while allowing
recursive schemas that descend into child values. These guards are not a
complete budget for native schema compilation or validation; service deployments
still need process memory and time limits.

`SapioHostAPI<T, R>` resolves a locator and provides typed calls. The old
`SapioJSONTrait` crate and its example-based construction check have been removed;
custom arguments need `Serialize + JsonSchema + Clone`, without an example
implementation. Results need `Deserialize + JsonSchema`. Existing versioned enum
tags remain the calling convention, and a receiver may accept additional
variants. The validator is a host dependency and adds no browser imports to
standalone WASM guests.

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
16 points per requested element. An exhausted allowance traps execution. Failed
exported calls identify fuel exhaustion using the runtime's metering state and
report the module ID, export and fuel limit; other traps retain their original
error. Threads
are disabled so atomic waits cannot block outside the fuel accounting.
Declared memory/table maxima below the host caps remain in effect.

Nested calls reserve an attempt before loading a module, including failed
lookups. Dropping a child does not restore attempts. A new top-level handle or
`fresh_clone()` receives independent fuel, memory and nested-call allowances;
ordinary calls on an existing handle do not reset them. Allocating host callbacks
cannot recursively reenter the guest allocator. Compilation receives an explicit
public `LoweringPlan` through `context.lowering`. The WASM host exposes no signer
selection or PSBT-signing imports; guests derive supported covenant policies
locally from those public inputs. See [covenant lowering](ENFORCEMENT.md).

These are guest execution limits, not a wall-clock deadline or a process memory
limit. Native compilation and host serialization/schema work are outside
instruction metering. Signer I/O occurs separately during binding and signing,
with its own request deadlines below.
Wasmer may reserve substantially more virtual
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

## Language core

The [action semantics](LANGUAGE_SEMANTICS.md) describe declarations, conditional
compilation, guard caching, metadata, effect paths and transaction deduplication.
Unknown macro options are errors. Cached guards now take only `self`; their
metadata still receives each attachment's context. Multi-guard conjunctions
retain inscription effects and use valid Miniscript arity.

Each action's policy is extracted before deduplicating its transaction. Compiled
templates retain their complete alternative preconditions. Sharing a commitment
requires identical funding requirements, metadata and child graphs; conflicts
report the affected field and action/effect path. Reuse a common compiled child
and metadata when intentionally returning the same transaction from multiple
actions.

Custom languages implement `PolicyCompiler` and use `#[guard(policy)]` or
`Builder::add_policy`. The [policy backend guide](POLICY_BACKENDS.md) describes
ordered script composition, lowering budgets, checked Taproot artifacts,
external witness construction and the artifact schema migration. Opaque scripts
have no inferred satisfaction-weight bound; a requested minimum feerate is an
error for these contracts.

Native policies are validated before simplification. Fixed transaction templates
are checked against their known CLTV, CSV and CTV requirements. Taproot keys and
identical complete scripts are selected deterministically; raw fragment and
inscription order within a script remain significant.

## Template funding and size

Compiled contracts expose `required_input_amount`, serialized as integer
`required_input_amount_sats`. It covers the `ensure_amount` floor and all
committed and suggested template requirements. The builder rejects underfunded
child outputs; compilation validates fresh and reused artifacts; binding checks
the explicit minimum even for finish-only contracts. The
[funding guide](FUNDING.md) explains auxiliary contributions and migration from
`amount_range`. Address and descriptor constructors now take an explicit
`Amount` minimum; use `Amount::ZERO` for an unrestricted destination.

A builder debits funds only when recording an output or explicit fee. Calling
`add_fees`, including with zero, closes output construction. Repeated calls add
to the recorded fees. `Context` remains a lower-level construction API; the
builder's accounting assumes its supplied context accurately describes the
input and any tracked ordinal ranges.

`unsigned_tx_size()` returns exact current serialized bytes, including every
output's actual script and CompactSize prefixes. `unsigned_tx_size_with_output`
includes a prospective output and any growth of the output-count prefix. Both
exclude the SegWit marker/flag and future scriptSig/witness data. They replace
`estimate_tx_size`, which omitted descriptorless scripts and counted a
nonexistent witness marker. The Hanukkiah and James vault examples now reserve
fees from complete unsigned sizes; this still does not bound signed fees.

## Emulator service limits

Each HD client signing exchange has a 30-second elapsed allowance by default,
including its wait for the connection mutex, connection setup, complete framing
and response validation. The socket is cached only after a complete response
passes the signature-additions check. Timeout, cancellation during an exchange
or an invalid response drops that socket; the next request reconnects. A queued
request that times out does not disturb the preceding request's connection.

The CLI's `covenant.request_timeout_secs` configures that allowance and a
separate deadline for awaiting resolution of the whole peer configuration; it
defaults to 30 seconds. System resolver work can continue after this async wait
times out and delay runtime shutdown. Direct library users configure exchanges with
`HDOracleEmulatorConnection::with_request_timeout` and must bound the
constructor's DNS resolution themselves. A federation applies the deadline to
each peer in its sequential signing loop. N peers can therefore consume N times
the configured request allowance across their exchanges.

`HDOracleEmulator::new(root)` admits at most 64 live connections with a 30-second
request allowance. `with_limits(request_timeout, max_connections)` overrides
both values. At capacity the listener stops accepting; the operating system's
backlog wait is outside the server deadline. For an admitted connection, each
deadline covers idle time, the complete header/body and the response write.
Dripping bytes cannot extend it; completing a response starts the next request's
allowance. Peer errors close only their connection. The listener owns and reaps
its connection tasks, and cancelling it aborts the active tasks.

These I/O deadlines cannot preempt synchronous signing, JSON parsing or response
validation. Native CPU budgets, process memory and deployment-level admission
policy remain separate work. The tests use paused time for stalled frames,
trickled responses, queue waits and blocked writes, plus loopback sockets for
reconnection, server admission and cancellation. See the
[emulator guide](../ctv_emulators/README.md) for the structural signing rule and
configuration.

`ProgramOracle` and `ProgramClient` provide a separate versioned evaluated
signing protocol with the same frame bound and default 30-second I/O allowance.
The program server admits 64 connections by default. Each client request uses
a fresh connection and has no automatic retry or CTV fallback. Program bytes,
preset parameters and auxiliary witness each have a 65,536-byte bound. The
operator registers exact WASM interpreters, or uses inline WASM through the zero
evaluator ID. Guest execution shares bounded fuel with native crypto imports;
elapsed deadlines cannot interrupt native module compilation. See the [protocol and signing
limits](PROGRAM_EMULATION.md#protocol-and-resource-limits).

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

Each template now requires `required_input_amount_sats`, the funding needed from
contract input zero after declared external contributions. `max_amount_sats`
continues to cover aggregate outputs plus reserved fees. Object amount ranges
refer to the contract input, so a buyer's payment does not inflate an NFT's own
funding requirement. Binding checks a known input zero independently, even if
auxiliary inputs remain unresolved. Recompile old artifacts to obtain the new
mandatory field; there is no guessed default for missing funding metadata.

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
[fork-inscriptions]: https://github.com/sapio-lang/rust-miniscript/blob/d9f9176a68a93efbfb303f24da154f9c72829892/SAPIO_EXTENSIONS.md
