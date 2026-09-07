# Sapio modernization plan

This is a recovery and development plan for the checkout based on `1933801`.
It covers a maintained toolkit, a covenant research platform, and a better
language experience. Those should share a compiler and artifact format. Building
three independent products would multiply the maintenance problem.

## What “respectable” means

A new developer should compile a meaningful contract from a clean checkout,
inspect its assumptions and resulting transactions, understand a failure without
reading compiler internals, and reproduce the result from a released version.
A maintainer should be able to review a small change, run a relevant test, and
explain what would make the resulting contract unsafe.

A supported release needs all of these gates:

- Reproducible stable Rust builds, tested native and WASM paths, maintained CI.
- Correct transaction commitments, amounts, signing indices and fee accounting.
- Explicit enforcement assumptions attached to artifacts and checked by tooling.
- A documented, bounded module interface with real compatibility checks.
- One working tutorial each for authoring, inspecting, and signing a contract.
- A versioned artifact schema, documented supported APIs, and release ownership.
- Security review of consensus-facing calculations, signers, and module hosting.

A prettier README or a dependency bump alone does not meet those gates.

## Baseline findings

The project has useful substance: Rust contract traits and macros, transaction
DAG compilation, Taproot/PSBT support, CTV emulation, a module ABI, and a substantial
contract corpus. Preserve that working knowledge. The original checkout also
contained correctness defects, incomplete checks, broken setup instructions,
and excluded integration coverage.

The baseline depended on the Sapio forks of Bitcoin 0.28 and Miniscript 7,
Wasmer 4, a Clap 3 beta, and an old JSON Schema validator. The forks implement
CTV-specific policy/compiler/interpreter behavior. Replacing their package names
with upstream crates would remove semantics, not complete a migration.

### Implemented

| Area | Change and evidence |
| --- | --- |
| Stable builds | Rust 1.98.1 pin, explicit compiler minimum, both dependency locks, resolver 2, removal of the nightly associated-type default |
| Linux runtime compatibility | Upgrade Wasmer and its cache to 6.1.0, which provides the stack probe removed from Rust's x86 runtime; remove unused direct CLI runtime dependencies |
| PSBT signing | Use the selected input index for key and script paths; reject sighash errors; verify signatures independently on a two-input transaction |
| Artifact binding | Validate the complete object graph before signing or indexing: unsigned input-zero templates, matching commitments, descriptors, output metadata and funding totals; reject invalid auxiliary-input mappings |
| PSBT structure | Reject empty-input transactions, mismatched input/output maps and populated unsigned scriptSigs/witnesses before signing or finalization; preserve the PSBT on structural rejection |
| CTV hashing | Include nonempty scriptSig commitments; match 16 hash results from four unmodified BIP-119 vectors covering scriptSig and witness combinations |
| Compiler termination | Advance duplicate-action suffixes; compile a contract registering one action three times |
| Fees | Enforce the strongest requested minimum in virtual bytes against that template's reserved fees; reject overflow and unknown extra-input weights |
| Ordinal allocation | Preserve allocated/remaining range prefixes and suffixes; reject malformed and insufficient ranges |
| WASM host | Bound ABI messages, validate every memory read/write, bound string scans, propagate errors, and test hostile guests through Wasmer; register guest callbacks once with `OnceLock` |
| Emulator protocol | Bound both directions of framed JSON, discard failed streams, reject malformed PSBT maps, support prebound listeners |
| Integration | Restore the suite to the workspace; compile, sign and finalize two contract steps and reject a modified output |
| Developer checks | Formatting, real feature checks, native tests, all WASM example builds, CLI smoke checks, and API documentation |

The [development guide](DEVELOPMENT.md) gives reproducible commands. This list is
an implementation record, not a production-readiness claim.

## Architecture to develop

Jeremy's [A Calculus of Covenants](https://rubin.io/bitcoin/2022/04/12/calc-cov/)
provides the right organizing model: a family of intended transitions, a generated
verifier, a prover that can satisfy it, evidence that their accepted transitions
agree, and explicit assumptions. His discussion principally models local
single-coin covenants; multi-coin coordination needs additional reasoning.

Apply that model to the existing compiler in this order:

```text
Rust contract authoring
        |
        v
Validated transaction graph + named transitions + explicit assumptions
        |                              |
        v                              v
Bitcoin script/template backend     Graph inspection and research checks
        |
        v
Versioned compilation artifact
        |
        v
UTXO binding -> PSBT generation -> signing/finalization
```

The graph belongs between authoring and Bitcoin lowering. Start by extracting
and validating the existing representation; do not invent a general-purpose VM.
Amounts, output ordering, lock times, commitments and transition identities need
precise types and validation. Source paths should survive lowering so errors can
point back to the contract and action that caused them.

Effects and continuation inputs are explicit compiler inputs. RPC state, wallet
policy, logging and scheduling must not silently influence an already declared
contract. Fee estimates are policy checks; signature and covenant commitments
are transaction correctness. Keep those boundaries visible in APIs.

### Enforcement and release boundaries

[BIP-119](https://github.com/bitcoin/bips/blob/master/bip-0119.mediawiki) specifies
the proposed CTV behavior. Its status must be checked for each supported chain;
a compiler's ability to emit an opcode is not evidence of chain enforcement.

Define backend capability information before adding another covenant primitive:

| Mode | Required claim in an artifact | Required validation |
| --- | --- | --- |
| Native CTV research | Exact opcode semantics and intended chain/deployment | Reference hash vectors and execution against a node implementing those semantics |
| Signer-emulated covenant | Signer identities, threshold, authorization policy and availability assumptions | Allowed transitions sign and finalize; forbidden transitions fail |
| Future primitive | Precisely named capability and assumptions | Its own lowering tests and execution evidence |

The current CLI does not yet enforce this artifact-level distinction. A release
must make backend selection explicit and fail before funding an unsupported
combination. Native CTV and signer emulation must not silently substitute for one
another. Never infer mainnet safety from a successful compilation or a unit test.

## Ordered work after this recovery pass

### 1. Finish correctness and hosting boundaries

This is the next release blocker, before a broad dependency migration.

- Repair the Sapio Miniscript fork's CTV paths following the
  [dependency audit](CTV_FORK_AUDIT.md). The local Sapio hash fix does not patch
  registry source. The fork needs nonempty-scriptSig hashing, checks against the
  fully finalized transaction, and defined scriptSig finalization ordering.
  Keep the builder's empty-scriptSig domain explicit; run the full official hash
  corpus and finalization tests before claiming general transaction support.
- Extend artifact boundary checks to funding UTXO identity and emulator responses.
  Structural graph and PSBT validation now run before binding, signing and
  finalization. Resolve graph path collisions without rejecting valid reused
  leaf objects; preserve transaction-index errors instead of treating every
  failed lookup as missing funding data.
- Replace the no-op `SapioJSONTrait` compatibility check. An example accepted by a
  schema is only a compatibility probe, not a proof of schema inclusion. Specify
  versioned interfaces and validate actual calls on both sides. Use an offline
  validator that works in standalone WASM without browser imports; bound schema
  work and reject unresolved references.
- Meter guest execution, total guest memory and cross-module depth. Bound cache
  input sizes and document/validate native compiled-artifact trust. A limit on
  one JSON transfer does not bound an entire compilation.
- Review emulator request timeouts, concurrency, authentication and authorization.
  A signature service reachable over TCP is not automatically a safe oracle.

Acceptance: malformed inputs fail with attributable errors; adversarial modules
terminate within configured limits; the hash, script, signing and finalization
layers agree on the supported transaction domain.

### 2. Modernize dependencies in semantic groups

Use a dependency inventory and advisory scan to select exact target versions at
the time of each migration. Keep one reviewed lockfile change per coherent group.
Do not mix a Bitcoin data-model migration, runtime replacement and language
rewrite in one change.

1. Move Clap to a maintained stable release. Preserve intentional command
   behavior with CLI tests, replace panic paths, fix exit codes and version
   reporting, and separate human diagnostics from JSON output.
2. Replace the JSON Schema validator and align the declared schema draft across
   the CLI and module interfaces. Exercise malformed and recursive schemas.
3. Evaluate further WASM runtime upgrades after implementing the resource contract.
   Benchmark compile time and memory, test cache invalidation across runtime
   versions, and run real Rust modules as well as small adversarial fixtures.
4. Port the CTV extension onto maintained Rust Bitcoin/Miniscript APIs, or maintain
   a small explicitly owned extension if upstream extension points are inadequate.
   Inventory policy ASTs, encoding/decoding, satisfaction, interpretation and PSBT
   finalization before porting. Preserve the golden transaction corpus.
5. Upgrade procedural macro parsing and remove dead dependencies and feature
   combinations. Enforce warnings on the supported core after clearing its debt.

Acceptance: no unexplained fork delta, no ignored actionable advisory without an
owner and rationale, reproducible native/WASM builds, and unchanged intended
transaction behavior or a documented deliberate change.

### 3. Give developers a coherent toolkit

Choose a small supported contract set first: a payment tree, a delayed recovery
vault, and a contract with a continuation. Treat the remaining examples as
research until they meet the same standards.

- Provide `new`, `check`, `compile`, `inspect`, `bind`, and `finalize` workflows
  with consistent arguments, errors, exit status, and machine-readable output.
- Explain a compilation with a transaction graph, amounts, fee budgets, timelocks,
  signer assumptions, and unresolved funding inputs. Start with deterministic
  JSON and a simple local viewer; share the artifact parser with the CLI.
- Ship explicit configuration examples for local emulation and a supported native
  research node. Remove dependence on obsolete public emulator defaults.
- Make book examples executable fixtures; clearly mark historical experiments.
  Every supported tutorial should run from a clean checkout in CI.
- Consolidate duplicated terminology and obsolete crate/server references.

Acceptance: an unfamiliar developer follows one documented path from an example
contract to inspected, signed, finalized transactions on the stated environment.
No undocumented sibling checkout, nightly feature or globally populated cache.

### 4. Develop the language around the graph

Keep the Rust embedded language as the first frontend. Stabilize the smallest
useful compiler API and improve errors before designing another syntax.

- Represent contract actions, guarded transitions, continuations and output
  amounts explicitly in a validated graph. Keep the Bitcoin backend separate.
- Add source/path-aware errors for out-of-funds, incompatible continuations,
  unreachable or conflicting actions, and unsupported backend capabilities.
- Define deterministic compilation: canonical inputs and effects, stable ordering,
  reproducible artifacts, and versioned semantics. Test repeated compilation and
  fixed golden examples before promising cross-platform byte identity.
- Write a compact semantic specification with worked examples and negative cases.
- Evaluate a standalone DSL only against concrete authoring problems that Rust
  macros cannot solve well. Any additional frontend targets the same graph.

Acceptance: the graph and artifact semantics can be understood without reading
proc-macro expansion, and a frontend refactor cannot silently change transactions.

### 5. Make research reproducible and maintenance durable

Build a catalog that pairs each covenant construction with intent sets, verifier,
prover, assumptions, composition limits, and executable positive/negative cases.
Add differential tests against reference scripts/nodes and targeted fuzzing at
parsing, lowering, linking, signing and ABI boundaries. Record the limits of
multi-coin and recursive constructions rather than assuming local proofs extend.

Before tagging a supported release, establish active maintainers, a private
security reporting route, supported platform/API policy, a changelog, release
checklist and reproducible release artifacts. Review the existing contribution
assignment terms with the rights holders; changing a README cannot change those
rights. Automate dependency review and keep changes small enough to bisect.

Acceptance: another maintainer can reproduce a release, diagnose a failed
invariant, and maintain the supported examples without recovering unwritten
knowledge from the original author.

## Scope discipline

The recovery commits are a foundation. They do not deliver a new standalone
language, formal correctness proofs, a hardened multi-tenant service, an audited
wallet, or a production deployment. Finish and review the release blockers above
before presenting Sapio as suitable for protecting funds.
