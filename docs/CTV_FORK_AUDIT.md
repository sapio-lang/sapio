# CTV hashing and finalization in the Miniscript fork

This audit records the remaining CTV boundary work after Sapio's local hash fix.
It is based on the dependency source used by the lockfiles, inspected on
2026-09-07. It does not establish which chains enforce CTV.

## Audited dependency

- Package: `sapio-miniscript 7.0.2-alpha.0`.
- Source revision: `3f23950459f3424ccfeecc0bb14579ec2aec9820`, recorded in the
  published crate's `.cargo_vcs_info.json`.
- Repository: [sapio-lang/rust-miniscript][fork].
- Hashing and public PSBT extension: [`src/psbt/mod.rs`][psbt].
- Finalization and interpreter checks: [`src/psbt/finalizer.rs`][finalizer].

The findings concern this pinned source. A later fork release must be reviewed
and tested before replacing it.

## Findings and affected paths

1. **The private hash helper omits scriptSigs.** `psbt::get_ctv_hash` at line 368
   serializes version and locktime immediately followed by input count. BIP-119
   requires an intervening hash of every serialized scriptSig when any is
   nonempty. Sapio's corrected `CTVHash` implementation does not change this
   dependency function.
2. **Satisfaction uses the incomplete helper.** The public
   `PsbtInputSatisfier::check_tx_template` implementation at line 362 extracts
   the transaction, including `final_script_sig` fields, but then calls that
   helper. A correct BIP-119 commitment for nonempty scriptSigs will not match.
3. **Interpreter checks hash the unsigned transaction.**
   `interpreter_inp_check` in `finalizer.rs`, line 320, passes
   `psbt.unsigned_tx` to the helper. Fixing only the helper still excludes
   finalized scriptSigs. This affects `PsbtExt` finalization, the public
   `psbt::interpreter_check`, and `PsbtExt::extract`.
4. **Sequential finalization does not recheck the complete result.**
   `PsbtExt::finalize_mut` at line 605 verifies and finalizes one input at a
   time. A later input can add a scriptSig and change the commitment of an
   earlier CTV input. Per-input verification alone cannot establish that the
   final transaction satisfies all CTV commitments. Correct positive-case
   behavior also needs a defined order for obtaining committed scriptSigs.
5. **Modern finalization bypasses the fork's PSBT sanity check.**
   `finalize_mut` and `finalize_mall_mut` do not call `sanity_check`; the
   deprecated helper does. Callers must validate transaction/metadata lengths
   before these public paths reach indexed accesses. Sapio's
   `finalize_psbt_format_api` calls the modern `PsbtExt::finalize` path.

The [BIP-119 specification][bip119] describes the scriptSig commitment and the
case of a CTV input combined with a legacy input whose exact scriptSig is known.
That mixed-input case exposes the difference between hashing a template and
checking the fully satisfied transaction.

## Current supported domain and evidence

The maintained Sapio builder emits templates with empty scriptSigs. Keep that
restriction explicit at artifact boundaries while the fork repair is pending.
Empty unsigned scriptSigs alone do not establish that an arbitrary external
PSBT will retain empty scriptSigs when finalized.

Sapio's [hash test](../sapio-base/tests/ctv_hash.rs) checks sixteen expected
hashes from four official vectors against its local implementation. The fork's
interpreter test supplies a hash directly, which cannot detect these defects.
The [integration test](../integration_tests/tests/integration_test.rs) verifies
two signer-emulated steps with empty scriptSigs; it does not exercise native CTV
with mixed input types. General mixed-input CTV finalization remains unsupported.

## Owned repair sequence

1. Patch the maintained fork in its own repository, preserving the current
   Bitcoin data model for this change. Add the missing scriptSig commitment and
   validate the shared helper against the complete official hash corpus.
2. Make interpreter checks use the candidate transaction's actual scriptSigs,
   including the current candidate satisfaction. Define how finalization obtains
   all required scriptSigs before deciding whether a CTV branch is satisfiable.
3. Validate PSBT structure at the modern entry points. Check all inputs against
   the complete finalized transaction before reporting successful finalization;
   keep partial-finalization behavior explicit when a required scriptSig is
   unavailable.
4. Publish a reviewed release or pin its exact revision in both Sapio
   workspaces. Run native and WASM checks before updating support claims.

A local satisfier can override `check_tx_template`, and the public interpreter
accepts a caller-supplied hash. Neither replaces the private helper and finalizer
used by `PsbtExt`. Reimplementing finalization locally would duplicate substantial
fork logic; keep this repair in the owned dependency.

## Required regression evidence

- Run the [full official corpus][vectors] against Sapio's hash and the fork's
  helper, preserving every supplied index and expected result. The JSON starts
  with a descriptive string before its vector objects.
- Exercise the fork through `PsbtInputSatisfier::check_tx_template`, putting the
  vector scriptSigs in PSBT `final_script_sig` fields rather than the unsigned
  transaction. This checks the public satisfaction path as well as hash bytes.
- Finalize native CTV alongside a fixed legacy scriptSig in either input
  position. Require the permitted transaction to succeed and altered scriptSigs
  or outputs to fail, including when another input is finalized later.
- Check finalized transactions through `interpreter_check` and `PsbtExt::extract`,
  and retain empty-scriptSig coverage. Malformed PSBT metadata must return errors
  before indexing. These tests establish library behavior, not chain deployment.

[fork]: https://github.com/sapio-lang/rust-miniscript
[psbt]: https://github.com/sapio-lang/rust-miniscript/blob/3f23950459f3424ccfeecc0bb14579ec2aec9820/src/psbt/mod.rs
[finalizer]: https://github.com/sapio-lang/rust-miniscript/blob/3f23950459f3424ccfeecc0bb14579ec2aec9820/src/psbt/finalizer.rs
[bip119]: https://github.com/bitcoin/bips/blob/master/bip-0119.mediawiki
[vectors]: https://github.com/bitcoin/bips/blob/master/bip-0119/vectors/ctvhash.json
