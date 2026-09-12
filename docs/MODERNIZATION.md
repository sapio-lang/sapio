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
| Bitcoin dependencies | [Bitcoin 0.32.102, upstream secp256k1 0.29.1 and Miniscript 13.1.0](BITCOIN_032.md); preserve CTV, inscriptions and strict PSBT/signature parsing while moving amount/schema types into Sapio |
| Stable builds | Rust 1.98.1 pin, explicit compiler minimum, both dependency locks, resolver 2, removal of the nightly associated-type default |
| Linux runtime compatibility | Upgrade Wasmer and its cache to 6.1.0, which provides the stack probe removed from Rust's x86 runtime; remove unused direct CLI runtime dependencies |
| PSBT signing | Use the selected input index for key and script paths; reject sighash errors; verify signatures independently on a two-input transaction |
| Artifact binding | Validate the complete object graph before signing or indexing: unsigned input-zero templates, matching commitments, descriptors, output metadata and funding totals; reject invalid auxiliary-input mappings |
| Funding representation | Replace ambiguous BTC ranges with an explicit integer-satoshi minimum; retain `ensure_amount` without templates, validate fresh/reused graphs, reject underfunded children and enforce known input floors before signing |
| Template accounting | Make builder debits private, preserve outputs-before-fees ordering, and count exact unsigned bytes and prospective outputs; regressions cover CompactSize boundaries, ordinal prefixes and the affected fee-paying examples |
| Covenant assumptions | [Explicit CTV wrappers and public lowering plans](ENFORCEMENT.md), recorded predicate requirements including finish guards, pure compiled-object reuse checks and runtime signer compatibility; explicit CLI native assumptions before wallet funding |
| Program artifacts | Typed program branches survive lowering, serialization and key selection; explicit preparation validates source/descriptor correspondence and funded PSBT data; fresh-process fragment and eltoo spends work without optional SIMP metadata or contract objects |
| Program emulation | [Exact program instances and public oracle keys](PROGRAM_EMULATION.md), versioned evaluated requests, a signature-bound transaction view and verified responses limited to one input/path; a fixed payment predicate admits multiple continuation candidates through local and TCP signing |
| Funding integrity | Authenticate previous transactions and index acknowledgements; check contract scripts, distinct inputs, checked funding totals and reserved fees before signing; preserve operational lookup errors and authenticated PSBT prevouts |
| Bound graph identity | Derive child keys from parent bindings and transition/output identities; preserve reused leaves and contracts, original source paths and continuation paths; keep synthetic funding in its own path |
| PSBT structure | Reject empty-input transactions, mismatched input/output maps and populated unsigned scriptSigs/witnesses before signing or finalization; preserve the PSBT on structural rejection |
| CTV hashing | Include conditional commitments to every serialized scriptSig in Sapio and the pinned Miniscript fork; both tests cover all 400 expected hashes from the complete official BIP-119 corpus |
| CTV finalization | Pin fork revision `04b69f69459fe3b043ca61fb649cf546d5a241b6`; establish legacy scriptSigs before native witness inputs, verify candidate scriptSigs and the completed transaction; regressions cover native WSH/Taproot with legacy inputs in either position |
| Finalizer metadata | Validate PSBT structure and referenced non-witness output bounds; honor explicit ECDSA/Schnorr sighash types on partial and finalized signatures, permit valid non-ALL signatures when no type is declared |
| Compiler termination | Advance duplicate-action suffixes; derive guard metadata beneath each guard branch and propagate errors; regressions compile repeated actions and a contract with two guards |
| Language core | [Action semantics](LANGUAGE_SEMANTICS.md): strict macro options and trait interfaces, context-free cached clauses with per-attachment metadata, stable condition slots, original action authorization before transaction deduplication, conflicting binding payload errors and inscription-aware guard composition |
| Policy lowering | Validate source before simplification, reject fixed templates incompatible with known native CLTV/CSV/CTV requirements, and canonicalize bare-key selection and identical leaf scripts |
| Custom policy languages | [PolicyCompiler API](POLICY_BACKENDS.md), checked ordered raw fragments, bounded expansion, reconstructed Taproot spending proofs, a real WASM guest and explicit external-finalization diagnostics; carries forward PR #269 |
| Inscriptions | Repair fork parsing, script-byte preservation, resource/key analysis and interpreter support; 51 fork inscription tests plus Sapio plugin artifact/signing and WASM checks, with checked ordinal ranges and fees |
| Inscription node validation | Bitcoin Core 31.1 accepts five library-finalized ordinary Taproot reveals and rejects 21 invalid variants in isolated regtest checks; native CTV and Ord index/sat assignment remain separate |
| Fees | Enforce the strongest requested minimum in virtual bytes against that template's reserved fees; reject overflow and unknown extra-input weights |
| Ordinal allocation | Preserve input order and range prefixes/suffixes; check target offsets, payout conservation and fees; separate original sats from external funding |
| WASM host | Bound ABI messages, validate every memory read/write, bound string scans, propagate errors, and test hostile guests through Wasmer; register guest callbacks once with `OnceLock` |
| WASM execution | Meter start and every guest call, charge variable-size memory/table operations, cap accessible memory and tables, disable threads, and bound nested module attempts/depth and allocator reentry |
| WASM source cache | Limit binary sources to 128 MiB, authenticate content hashes, recompile with the current engine, preserve corruption/I/O errors and ignore legacy native caches |
| Module schemas | Validate actual inputs and successful outputs at the common host boundary, including raw nested calls; use offline Draft 7 validation, guard reference expansion and remove the no-op interface check |
| Emulator protocol | Bound both directions of framed JSON, discard failed streams, reject malformed PSBT maps, support prebound listeners |
| Emulator responses | Accept complete PSBT responses containing only signature additions; preserve existing signatures and every other field in raw HD responses and each federation participant; signer callbacks are absent from the compilation host |
| Emulator lifecycle | Bound each peer exchange and the async wait for CLI peer resolution; cap admitted server connections; discard incomplete exchanges, isolate peer errors and cancel owned connection tasks on shutdown |
| Integration | Restore the suite to the workspace; compile, sign and finalize two contract steps and reject a modified output |
| Contract examples | [Complete inventory](EXAMPLES.md): repaired and tested library families, restored PowSwap/TapBet, 20 real WASM fixtures, both native examples and explicit research assumptions |
| Developer checks | Formatting, feature checks, native tests, complete WASM catalog with artifact/schema/repeatability checks, CLI smoke checks, and API documentation |

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

The current [enforcement boundary](ENFORCEMENT.md) lowers explicit
`Emulatable(Ctv(...))` predicates using a serialized public `LoweringPlan`.
Compilation has no signer runtime or host signing callback. Artifacts record
the plan and every resolved predicate, including finish guards. Reuse checks
public plans without network access; binding compares derived requirements with
the actual signer policy across the graph, including suggested descendants.

The CLI separates compilation inputs from runtime native research,
signer-emulation or combined configuration. Ordinary signer emulation rejects
known native CTV instructions before wallet funding; the combined mode supports
mixed policies with both signer-policy checks and an explicit native activation
assumption. Neither errors nor missing configuration silently select another
backend. Ordinary native clauses and raw scripts are never rewritten.

These checks do not detect chain activation, prove signer availability, identify
operators or authenticate arbitrary artifact policies against their scripts.
Native research remains an explicit operator assumption; address-only outputs
reveal no spending script to inspect. Before supporting deployments or another
covenant primitive, establish the remaining evidence:

| Mode | Required claim in an artifact | Required validation |
| --- | --- | --- |
| Native CTV research | Exact opcode semantics and intended chain/deployment | Reference hash vectors and execution against a node implementing those semantics |
| Signer-emulated covenant | Signer identities, threshold, structural signing rule and availability assumptions | Allowed transitions sign and finalize; forbidden transitions fail |
| Future primitive | Precisely named capability and assumptions | Its own lowering tests and execution evidence |

The recorded predicate requirements are narrower than a complete deployment
capability declaration. Future continuation behavior and supported chain
semantics need their own evidence. Never infer mainnet safety from a successful
compilation or a unit test.

The [generic encumbrance boundary](ENFORCEMENT.md#generic-encumbrance-programs-design-boundary)
now implements versioned instance commitments, domain-separated public
derivation and evaluated signing requests through bounded WASM evaluators.
The zero evaluator ID executes inline WASM; other IDs commit to registered
WASM interpreters. A payment example admits different amounts and output orderings
under one fixed continuation policy. Generic source and signer assumptions are
retained explicitly by the application; they are not inferred from ordinary
keys or dispatched by CTV artifact checks and CLI modes. A compiled CTV guest
uses the shared metered SHA256 host API, and compiler guests delegate public
BIP32 derivation to native host code. Automatic generic artifact dispatch,
BitVM disputes and penalty bonds remain future work.

## Ordered work after this recovery pass

### 1. Finish correctness and hosting boundaries

This is the next release blocker, before a broad dependency migration.

- Extend [the binding checks](BINDING.md) with explicit chain/wallet funding
  policy. Transaction identities, known amounts, scripts, graph paths and
  emulator response integrity are checked; unresolved offline inputs remain
  possible. Establish confirmation/unspentness where required and keep that
  policy separate from artifact validation. The repaired fork verifies against
  supplied prevouts; callers outside the binder must also authenticate them and
  reject conflicting UTXO records.
- Extend the explicit [covenant assumptions](ENFORCEMENT.md) with deployment
  evidence and execute the supported native CTV cases against a node implementing
  the intended semantics. Exact recorded-policy checks and native-script
  detection are implemented; automatic chain-activation detection and a complete
  capability model are not. The
  [fork repair record](CTV_FORK_AUDIT.md) documents hashing, finalization and
  explicit sighash checks, including 157 passing fork tests and Clippy. Keep the
  builder's unsigned, empty-scriptSig, input-zero domain explicit. Automatic
  finalization ordering covers the tested native WSH/Taproot and legacy cases;
  circular P2SH commitments, additional bare descriptors and arbitrary CTV input
  combinations need their own design and execution evidence.
- Specify the semantics and evolution of versioned module interfaces. Actual
  inputs and successful outputs are now checked against advertised Draft 7
  schemas at the host boundary. This does not prove behavioral compatibility or
  schema inclusion. The offline validator rejects unresolved references and
  excessive expansion; complete native validation work budgets remain open.
- Extend the [guest execution limits](DEVELOPMENT.md) with service-level
  compilation deadlines, process memory and concurrency policy. Guest fuel,
  memory/table caps, nested-call bounds and authenticated source caching are
  enforced. Emulator I/O now has elapsed request deadlines and bounded admitted
  connections; native compilation, schema work and cryptography still need
  CPU and process-memory budgets. Benchmark real contracts before changing
  the fixed allowances.
- Establish deployment and backend policy for emulator services, including key
  custody, availability and operator admission controls. The existing HD signing
  rule derives a key from the transaction's input-zero CTV hash and signs with
  `SIGHASH_ALL`; its covenant restriction is structural and does not depend on
  authenticating a requester. The [service limits](DEVELOPMENT.md#emulator-service-limits)
  bound I/O waiting and admitted connections, without establishing a hard CPU
  limit or proving a public deployment's availability. The separate program
  service evaluates an exact instance over signed transaction fields before
  signing its selected input/path. Its inline and registered evaluators run
  in fresh WASM instances with shared instruction/host-operation fuel and
  memory limits. Native module compilation remains outside execution fuel.

Acceptance: malformed inputs fail with attributable errors; adversarial guest execution
traps at the documented bounds; the hash, script, signing and finalization
layers agree on the supported transaction domain.

### 2. Modernize dependencies in semantic groups

Use a dependency inventory and advisory scan to select exact target versions at
the time of each migration. Keep one reviewed lockfile change per coherent group.
Do not mix a Bitcoin data-model migration, runtime replacement and language
rewrite in one change.

1. Move Clap to a maintained stable release. Preserve intentional command
   behavior with CLI tests, replace panic paths, fix exit codes and version
   reporting, and separate human diagnostics from JSON output.
2. Keep the host's JSON Schema validator and Draft 7 declarations aligned when
   upgrading Schemars. Preserve malformed, recursive and nested-call coverage;
   keep native validation dependencies out of standalone guests.
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

The full [example catalog](EXAMPLES.md) now has executable regression coverage.
Choose a smaller supported release set with explicit chain enforcement and end-to-end
spending evidence; compilation tests alone do not promote research constructions
to supported financial products.

- Provide `new`, `check`, `compile`, `inspect`, `bind`, and `finalize` workflows
  with consistent arguments, errors, exit status, and machine-readable output.
- Explain a compilation with a transaction graph, amounts, fee budgets, timelocks,
  signer assumptions, and unresolved funding inputs. Start with deterministic
  JSON and a simple local viewer; share the artifact parser with the CLI.
- Maintain explicit configuration examples for local emulation and establish a
  supported native research node. The CLI now requires a covenant mode without
  an implicit native fallback; deployment instructions must state its assumptions.
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
  Conditional paths now retain declared slots, guard metadata ordering no longer
  depends on allocation addresses, and duplicate transactions retain their
  authorization alternatives. Changes to the action factory list can still
  change renamed action paths; arbitrary Rust callbacks can observe external
  state. These remain explicit limits on reproducibility.
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

Before publishing supported Sapio crates, publish the reviewed Miniscript repair
and update the declared dependency requirements. The current workspace Git patch
does not propagate to external application or plugin workspaces.

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
