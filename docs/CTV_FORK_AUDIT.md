# CTV hashing and finalization in the Miniscript fork

This record describes the repaired CTV hashing and PSBT finalization boundary.
It establishes tested library behavior for the domain below. Native CTV
enforcement, funding-data authentication and production readiness require
separate evidence.

## Dependency revisions

- Package: `sapio-miniscript 7.0.2-alpha.0`.
- Repaired source: `04b69f69459fe3b043ca61fb649cf546d5a241b6`, pinned in both Sapio
  workspaces.
- Historical source: `3f23950459f3424ccfeecc0bb14579ec2aec9820`, recorded in the
  published crate's `.cargo_vcs_info.json` and audited on 2026-09-07.
- Repository: [sapio-lang/rust-miniscript][fork].
- Repaired hashing and public PSBT extension: [`src/psbt/mod.rs`][psbt].
- Repaired finalization and interpreter checks: [`src/psbt/finalizer.rs`][finalizer].

The historical source omitted scriptSigs from its private CTV hash helper,
checked interpreter commitments against the unsigned transaction, and finalized
inputs sequentially without verifying the complete result. Its modern
finalization entry points also bypassed PSBT structure checks. Sapio's earlier
local hash fix did not change those dependency paths.

The pin applies to builds rooted in these two workspaces. External consumers
must repeat the root patch, as described in the [development guide](DEVELOPMENT.md).
A repaired registry release and updated dependency requirements remain necessary
before publishing supported Sapio crates.

## Implemented repairs

1. **Hash every committed scriptSig.** When any input has a nonempty scriptSig,
   the fork hashes every input's serialized scriptSig, including empty ones and
   their length prefixes, between the locktime and input-count commitments.
   `PsbtInputSatisfier::check_tx_template` uses this corrected helper and the
   final scriptSigs supplied in PSBT metadata.
2. **Verify the actual candidate satisfaction.** Interpreter checks construct a
   transaction with the known final scriptSigs and overlay the current input's
   candidate scriptSig before calculating CTV. A candidate that introduces an
   uncommitted scriptSig fails before it is installed in the PSBT.
3. **Obtain legacy scriptSigs before native witness covenants.** Whole-transaction
   finalization processes legacy and wrapped witness inputs before native WSH,
   WPKH and Taproot inputs. It then interprets every input against the complete
   finalized transaction before returning success. Previously finalized inputs
   are verified and can be reused without their cleared partial-signature maps.
4. **Validate PSBT structure before indexed processing.** Public finalization,
   interpretation and extraction paths reject empty-input transactions,
   mismatched input/output maps and populated unsigned scriptSigs or witnesses.
   A missing referenced output when using a non-witness UTXO returns an error.
5. **Enforce explicitly requested signature hash types.** Every supplied ECDSA
   and Schnorr partial signature must match declared `PSBT_IN_SIGHASH_TYPE`
   metadata. Interpreter constraints apply the same check to executed signatures
   of previously finalized inputs. Taproot `DEFAULT` is accepted, and absent
   metadata permits any otherwise valid signature hash type. Mismatches identify
   the input and declared/actual flags and fail before modifying that input.

The [BIP-119 specification][bip119] defines the conditional scriptSig commitment;
[BIP-174][bip174] requires a finalizer to honor an explicit sighash declaration.
The repair stays in the owned dependency rather than duplicating its private
finalization logic in Sapio.

## Regression evidence

Both Sapio's [hash test](../sapio-base/tests/ctv_hash.rs) and the fork's
[public satisfier test][hash-test] cover all 400 expected hashes from the 100
transactions in the [official corpus][vectors]. Every supplied index is retained,
including hash-only cases outside a transaction's input range. The fixture and
its upstream revision/checksum are recorded in each repository's test data.
The fork test supplies scriptSigs through PSBT `final_script_sig` fields.
Removing the scriptSig hash repair makes the first official vector fail.

The fork's [finalization regressions][finalization-test] cover native WSH and
Taproot CTV combined with a legacy input in either input position, using fixed
final scriptSigs and supplied signatures. They verify permitted transactions,
reject altered outputs and replacement valid legacy signatures, reject a later
scriptSig that invalidates an earlier commitment, and check candidate scriptSig
inclusion. Both malleable and non-malleable finalization paths are exercised.
Malformed metadata and out-of-range non-witness prevouts return errors without
panicking. Interpreter checks and extraction revalidate the completed result.

The [sighash regressions][sighash-test] use valid ECDSA and Taproot key/script-path
signatures. They accept matching `DEFAULT`, reject mismatched declarations on
partial and finalized inputs without mutation, check unused supplied signatures,
and accept valid non-`ALL` signatures when no type was declared.

The merged fork suite passes 157 tests with the stable feature set, including
51 inscription tests, and passes Clippy. Its separate Bitcoin Core 31.1 regtest
check accepts five library-finalized inscription reveals and rejects 21 invalid
variants. Those node checks use ordinary Taproot conditions; they do not execute
native CTV. The [inscription record](INSCRIPTIONS.md) describes their scope.

Sapio's native suite and WASM smoke checks are described in the
[development guide](DEVELOPMENT.md). Its existing covenant integration test
remains a signer-emulated two-step transaction test. Native CTV node execution
and maintained coverage-guided fuzzing still require separate work.

## Supported domain and remaining limits

Sapio's builder and binding boundary continue to require unsigned templates with
empty scriptSigs/witnesses and the contract input at index zero. Correct hashing
of external PSBTs does not expand that artifact domain.

The repaired finalizer supports the tested combination of native WSH or Taproot
CTV and established legacy scriptSigs. A commitment to a legacy input requires
its exact scriptSig to be obtainable. Automatic ordering does not solve circular
P2SH CTV commitments, expand the accepted bare-descriptor set, or establish
general CTV finalization for arbitrary input combinations.

Single-input finalization checks currently known scriptSigs. Another input can
change that commitment later; use `PsbtExt::extract` or `interpreter_check` after
finishing the transaction. Whole-transaction finalization can leave successfully
processed inputs finalized when another input fails; callers must handle the
returned errors and preserve the remaining partial PSBT.

The fork verifies scripts against supplied prevouts. It does not authenticate
funding UTXO identity or reconcile conflicting witness/non-witness UTXO records.
Sapio's binder now authenticates known funding transactions and checks emulator
responses, including responses returned to WASM guests; callers outside those
boundaries must establish the same invariants. Backend capability checks and
execution against a node implementing the intended CTV semantics remain release
requirements. The [host limits](DEVELOPMENT.md) cover guest fuel, accessible
memory, tables, nested calls and source-cache integrity. Process memory, native
compilation and external service deadlines remain hosting requirements.

[fork]: https://github.com/sapio-lang/rust-miniscript
[psbt]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/src/psbt/mod.rs
[finalizer]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/src/psbt/finalizer.rs
[hash-test]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/tests/ctv_hash.rs
[finalization-test]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/tests/ctv_finalization.rs
[sighash-test]: https://github.com/sapio-lang/rust-miniscript/blob/04b69f69459fe3b043ca61fb649cf546d5a241b6/tests/psbt_sighash.rs
[bip119]: https://github.com/bitcoin/bips/blob/master/bip-0119.mediawiki
[bip174]: https://github.com/bitcoin/bips/blob/master/bip-0174.mediawiki
[vectors]: https://github.com/bitcoin/bips/blob/ae747e2b909ab5dd32632ed3a8b09839193d53e3/bip-0119/vectors/ctvhash.json
