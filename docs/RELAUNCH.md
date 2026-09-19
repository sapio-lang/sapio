# Relaunch readiness

The developer kit establishes one maintained path from a generated Rust project
to an inspected artifact and a completed synthetic spend. It is a developer
preview, not a production-readiness or security-audit claim.

## What the developer path includes

- `sapio-cli new` supplies a complete manifest, lockfile, pinned Rust toolchain,
  contract source, runner, tests and exact evaluator bytes. The starter pins
  Sapio revision `88cb0daf23762d942bb5c2532b3d27f770066c43`; its top-level patches
  select the matching Bitcoin, Miniscript and schema dependencies.
- The [quickstart](QUICKSTART.md) and generated README cover authoring, raw
  artifact inspection, preparation, explicit local signing, verified response
  import and finalization. Contract logic is separate from demonstration keys,
  synthetic funding and tests.
- The starter's checks distinguish permitted payments, rejected underpayments
  and local fee constraints. The repository's broader native, WASM and isolated
  Bitcoin Core checks remain documented in [DEVELOPMENT.md](DEVELOPMENT.md).
- The [walkthrough check](../contrib/check_quickstart.py) generates an external
  Cargo project and executes the three shell blocks from its generated README,
  including successful completion and the expected evaluator rejection. Native
  CI includes this check on Linux and macOS, alongside the repository tests.

The release commit's CI results are the evidence for its tested platforms and
commands. A rendered book or successful compiler build alone does not validate
an end-to-end developer workflow.

## Remaining gates

| Gate | Current boundary and required decision or evidence |
| --- | --- |
| Supported release scope | Name a release version, supported platforms/toolchain, public API scope and maintenance policy. The starter pin is reproducible source, not a promise of indefinite API compatibility. |
| Dependency distribution | Decide whether the release formally supports pinned Git dependencies or ships reviewed dependency releases. External Cargo workspaces require the complete top-level patch table; transitive patches do not propagate. |
| Artifact and intent evolution | Document how stored artifacts, request schemas and spend intents are supported across releases, including explicit rejection or migration of incompatible data. Current validation does not create a migration policy. |
| Security review and reporting | Review consensus-facing calculations, key derivation, signing/finalization and guest-host boundaries; establish a public reporting and response process. Regression coverage is evidence for its assertions, not an independent audit. |
| Release delivery | Publish identifiable release artifacts with verified provenance and license notices. Source installation and an external-project workflow check are provided; record passing results for every platform the release claims to support and validate any additional distribution method. |
| Real funding and signer operations | Define the application's artifact trust, wallet input selection, confirmations/unspentness, key custody, oracle honesty and availability requirements. The starter deliberately supplies synthetic funding and public keys. |
| Native covenant claims | For each claimed deployment, establish that the chain implements the intended opcode semantics and run the corresponding execution checks. Ordinary Taproot emulation tests do not establish native CTV activation. |

These gates have no implied owner or completion date. Record decisions and their
validation with the release rather than presenting research capabilities as
deployment guarantees. [MODERNIZATION.md](MODERNIZATION.md) retains the longer
implementation history; [ENFORCEMENT.md](ENFORCEMENT.md) defines the current
construction and signer boundaries.
