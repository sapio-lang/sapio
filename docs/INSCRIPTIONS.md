# Inscription example validation

The Miniscript inscription audit found defects in parsing, witness extraction,
resource/key analysis, policy compilation/serialization and interpretation.
Both workspaces pin the repaired source merged into the fork's master branch.
Its [review record][fork-review] gives the exact supported field/encoding domain
and reference sources.

The fork's Rust suite passes 157 tests, including 51 inscription tests. Coverage
includes all eight authoring fields, empty values and push/chunk boundaries,
binary/text/Serde and descriptor commitments, generated malformed inputs, policy
analysis, and signed WSH/Taproot spending conditions. These bounded cases cover
the documented domain; they are not an exhaustive search of arbitrary scripts.

The Sapio example also exposed a compiler defect: compiling two guards repeatedly
derived the same metadata path inside an infinite iterator. Guard metadata now
belongs to each guard's branch, and derivation failures propagate as errors.
The regression verifies both keys and both guard metadata entries survive.

`InscribingStep` requires nonempty, forward ordinal ranges whose checked total
matches the context funds. It checks fees by subtraction before retaining the
inscribed sat and the example's existing padding. Empty/malformed ranges and
excessive fees return errors. These checks do not authenticate ordinal history;
callers still supply trustworthy funding and ordinal information.

The native plugin tests compile an owner-controlled inscription with a 521-byte
body, serialize and deserialize its artifact, bind it to a funding output, sign
its Taproot script path, finalize it, and inspect the extracted inscription.
Missing/wrong owner signatures fail, and the owner is not used as an internal
key that would bypass the inscription script. Separate regressions reject
malformed ranges, excessive fees, and oversized content types.

Run them with:

```sh
cargo test --locked --manifest-path plugin-example/Cargo.toml \
  -p sapio-wasm-ordinal-inscription
```

`contrib/sapio_wasm.sh` runs that native test target before building the guests.
Its CLI smoke check also creates the actual inscription plugin with a 521-byte
body, checking the encoded envelope, owner key, and reveal output. The existing
direct and cross-module checks remain in the same run.

The fork also has an independent Bitcoin Core 31.1 check using a temporary
regtest chain with networking disabled. Core accepts five library-finalized
reveals covering prefix/postfix envelopes, empty/chunked body and metadata,
multiple envelopes and a 2-of-3 condition. It rejects 20 variants with changed
signatures, inscription content, control blocks or outputs, plus a separately
signed invalid script with a 521-byte push inside an unexecuted envelope. The
release archive is pinned by its official checksum, and the dedicated fork CI
job fails on any acceptance or rejection mismatch. Reproduction commands are in
the [fork's validation guide][fork-review].

These node checks validate the ordinary Taproot reveals above. Native CTV node
execution, Ord indexing and sat assignment/reinscription behavior, and
authentication of supplied funding history still need independent evidence.

[fork-review]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/docs/INSCRIPTIONS.md
