# Depots: theft-proof self-custodial channels over a bespoke evaluator

This executable Sapio example implements the on-chain protocol of jlspc's
[Depots: Theft-Proof, Self-Custodial Bitcoin For Billions Of
Users](https://delvingbitcoin.org/t/depots-theft-proof-self-custodial-bitcoin-for-billions-of-users/2892)
(Section 3.1, Figures 1 and 2), using the existing program-oracle framework
and one new evaluator for the protocol's CTV+CSFS predicates.

```sh
cargo run --locked -p sapio_integration_tests --example depots
cargo test --locked -p sapio_integration_tests --test depots
```

The public contract is
[`Depot`](../sapio-contrib/src/contracts/depots/mod.rs) with immutable
[`DepotTerms`](../sapio-contrib/src/contracts/depots/terms.rs). The
[runner](../integration_tests/src/depots_example.rs) holds disposable keys,
sells channels, chooses coins and drives the oracle; the example prints a
complete resolution without broadcasting.

## The protocol

An operator funds a single output `F` backing probabilistic Lightning
channels. A channel is an operator **blind signature** `GuSig_i` over
`<Clm_Ui, Clm'_Ui, i, Salt_i, G_i>`: the channel's two claim-transaction
hashes, its index, a salt, and a secret guess in `0..P`. Buying a channel
worth `D/2` costs `D/2P`, its expected value. The depot's address commits
to **no user set**: selling a channel is pure signing, so one depot serves
any number of users with a constant on-chain footprint.

After expiry `E` the depot resolves:

- `F -> Targ`: the operator reveals a target `t` by signature (`TaSig`),
  forcing the fixed Target transaction.
- `Targ -> Hit`: any owner of a channel with `G_i = t` presents
  `(t, TaSig, GuSig_i, ...)` and forces the fixed 50/50 split into an
  insurance half (output 0) and a claim half (output 1). With no hit the
  operator sweeps after `tsd`.
- `Hit -> Clm_Ui`: after `tsd`, the claim half can only move to the exact
  transaction whose hash the operator blind-signed — a CTV whose hash
  arrives in the witness, authenticated by CSFS. The claim funds a
  settlement layer that the user and operator close after another `tsd`.
- **Burn**: two *distinct* guess signatures for the same target destroy
  either half. Multi-hits burn the depot, and an operator trying to steal
  the claim half with an arbitrary spend must reveal a second signature,
  arming the user's burn of the insurance half. This griefer penalization
  is what makes the depot theft-proof for both sides.
- `F -> Targ'`: if the operator withholds the target for `tsd` past
  expiry, anyone resolves the depot with the target fixed at zero.

## Emulation

The protocol requires CTV and CSFS (and benefits from PairCommit). The
[depots evaluator](../evaluators/depots/src/lib.rs) implements the paper's
composed predicates as one metered WASM program with four stages — target
reveal, hit, claim, burn — parameterized by the depot's identity, keys,
prime and expected template commitment. Each contract leaf is an
`EmulatedProgram`: an oracle-derived key that signs only after the
evaluator validates the evidence against the BIP341-committed transaction
view. The default branch reuses the stock `templatehash` fragment, and
sweeps and settlements are ordinary Miniscript with native timelocks.

Reveals travel as off-chain evidence to the oracle instead of on-chain
witness data; a withholding operator is covered by the default branch
exactly as in the paper. Other modeling choices, documented in the module:
PairCommit nesting becomes a tagged hash, the salt is caller-chosen bytes,
burns are OP_RETURN outputs rather than twenty-year locks, the channel
interior is a delayed 2-of-2, and the paper's multi-target vectors and
randomization factors (Section 3.2) are off-chain refinements left out.

Fees come from a named sponsor input on every resolution transaction, so
depot value is preserved exactly as the paper assumes. This remains
covenant emulation under Sapio's oracle trust assumptions: Bitcoin
enforces the signatures, Taproot proofs and timelocks; the oracle enforces
the depot predicates before signing.

## What the tests check

The [end-to-end tests](../integration_tests/tests/depots.rs) drive the
oracle through: a full `F -> Targ -> Hit -> Clm` resolution whose claim
candidate equals the purchase-time blind-signed hash; rejection of misses,
wrong-target signatures, tampered tuples and another user's claim theft;
double-hit burns of both halves (and rejection of a replayed identical
signature); the operator-free default branch including its `Clm'`
binding; and channel-independence of the depot address as purchases
accumulate.
