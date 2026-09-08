# Inscription example validation

The Miniscript inscription audit found defects in parsing, witness extraction,
resource/key analysis, policy compilation/serialization and interpretation.
Both workspaces pin the repaired source. Its [review record][fork-review] gives
the exact supported field/encoding domain and reference sources.

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

This establishes the tested library and example flow. Bitcoin node execution,
Ord indexing/sat assignment and authentication of supplied funding history still
need independent integration evidence.

[fork-review]: https://github.com/sapio-lang/rust-miniscript/blob/4b30433f0b64f374a0314be25aafd32d1c0218a8/docs/INSCRIPTIONS.md
