# Contract examples

This inventory covers every contract family in `sapio-contrib`, all 19 WASM
modules and both shared interfaces in `plugin-example`, and the two native
executables. The examples demonstrate contract construction and have behavioral
regressions for their supported paths. They remain research examples: compiling
an artifact does not establish a deployed protocol, a complete wallet, or an
independent security audit.

The examples use the repository's CTV research semantics. Native demonstrations
and the WASM catalog use Regtest without funding or broadcasting transactions.
Read [development setup](DEVELOPMENT.md), [transaction binding](BINDING.md), and
[inscription support](INSCRIPTIONS.md) before extending those paths.

## Shared assumptions

- Monetary units follow each field's schema. Context funding, template amount
  fields and `AmountU64` use integer satoshis. `AmountF64` and fields using
  `as_btc` still accept BTC amounts; `CoinAmount` uses its tagged representation.
  Contract arithmetic uses integer `Amount`
  values after deserialization; this update does not migrate every JSON field
  to satoshis.
- `Template.max`, serialized as `max_amount_sats`, includes outputs and reserved
  fees across all inputs. `required_input_amount_sats` is the separate minimum
  for input zero. Buyer funding must not inflate the NFT or contract's own
  funding requirement. Nonzero external funding requires an auxiliary input,
  and all original and external funds are added with checked arithmetic.
- Keys serialize as strings; native key parsing checks their validity. A valid
  schema does not establish that a key belongs to the intended person, that an
  oracle reports honestly, or that a participant will sign.
- Timelock types enforce their encoded domains. Relative time values retain the
  BIP68 type bit and 512-second units; converting a duration rounds up. Height
  and time constraints can only be combined where the transaction format allows
  it.
- These examples do not implement a fee estimator or a general dust policy.
  Declared fees are checked against the available budget. Small outputs in
  tests exercise accounting and need not satisfy a node's relay policy.
- Finite trees, schedules and outcome grids require work proportional to the
  generated contract graph. Termination checks reject parameters that cannot
  make progress; they do not make arbitrarily large valid graphs inexpensive.

## Library contracts

Sources are under [sapio-contrib/src/contracts](../sapio-contrib/src/contracts).
Some teaching models keep their implementation types private; their inline tests
show how to construct and exercise them.

| Family | Supported behavior and repairs | Behavioral coverage and assumptions |
| --- | --- | --- |
| [Basic examples](../sapio-contrib/src/contracts/basic_examples.rs) | Key and timeout guards, state-dependent quorum, and conditional compilation. The timeout still requires Bob's key; participant counts no longer truncate to eight bits. | Compiled guard structure, state transition, 256-participant counting, invalid quorum, omitted skippable branches, required branches, and conflicting/error conditions. |
| [README contracts](../sapio-contrib/src/contracts/readme_contracts.rs) | Public-key payment, two equivalent escrow formulations, and a delayed predetermined split. Both escrow formulations require the escrow key plus either party. | Key/escrow guards, exact split destinations and values, timeout sequence, and underfunding. |
| [Channel](../sapio-contrib/src/contracts/channel.rs) | A two-party cooperative settlement and a contest-to-resolution teaching model. Settlement balances must match the channel; the delayed resolution transaction carries its 100-block sequence. | Nested artifact compilation, CSV guard/sequence agreement, both parties' settlement authorization, exact balances, absent updates, malformed balances, underfunding, and database locator round-trip. The old empty revocation/update placeholders were removed; this is not a revocation protocol. |
| [Eltoo channel](../sapio-contrib/src/contracts/eltoo_channel.rs) | A prototype of ordered updates, minimum settlement maturity, and cooperative payout. Initial updates use a valid timestamp domain; successor overflow fails closed. | Initial and newer updates, stale updates, balanced/nonempty resolution, minimum maturity, cooperative outputs, and sequence exhaustion. This does not supply an ANYPREVOUT implementation or a complete eltoo protocol. |
| [Coin pool](../sapio-contrib/src/contracts/coin_pool.rs) | Cooperative updates and recursive exits with one refund per participant. Refund totals and external funding are checked. | Odd-sized pool splitting, conserved exit amounts, authorization threshold, malformed/empty refunds, overflow, and undeclared auxiliary funding. Pool coordination and agreement on updates are external. |
| [Dynamic contracts](../sapio-contrib/src/contracts/dynamic.rs) | Two forms of runtime contract construction create spendable key-controlled children. Odd balances split without loss. | Actual compiled children and output values, artifact validation, and rejection below the two-satoshi minimum. |
| [Federated sidechain](../sapio-contrib/src/contracts/federated_sidechain.rs) | Normal federation quorum and a recovery state requiring a separate quorum after 4,725 blocks. | Recovery funding, key and delay guards, terminal recovery state, invalid quorums, underfunding, and the exact catalog fixture with a key in both normal and recovery groups. No sidechain validation or federation networking is implemented. |
| [Hanukkah](../sapio-contrib/src/contracts/hanukkah.rs) | Eight sequential nights or eight independently scheduled night outputs, with 36 candle payments in total. Every transaction reserves its own fees from actual funding. | Both layouts, all candle counts, daily locktimes, nonzero fees, the 36-address string round-trip, malformed recipient counts, invalid nights, and timestamp/fee overflow. |
| [Hodl chicken](../sapio-contrib/src/contracts/hodl_chicken.rs) | A two-party commitment game with equal deposits and checked, conserved payouts. Validated deserialization replaces recursive Serde conversion. | Flat JSON round-trip, both winner/loser destinations and amounts, unequal deposits, underfunding, and overflow. |
| [OP_RETURN chain](../sapio-contrib/src/contracts/op_return_chain.rs) | Signed data publication followed by continuation or redemption. Fees are deducted before allocating change. | OP_RETURN payload, signer guard, output values, reserved fee, full compilation, oversized data, and insufficient fee funding. The example's payload bound is part of its API. |
| [Staked signer](../sapio-contrib/src/contracts/staked_signer.rs) | A signing key can burn the stake; a separate redemption key can enter a delayed closing state. | Burn destination and amount, both keys' guard roles, closing output, 20-block redemption delay, and the retained burn path while closing. Leakage detection and off-chain signing are external. |
| [Taproot bet](../sapio-contrib/src/contracts/taproot_bet.rs) | A historical activation-bet example with positive recurring payouts, per-step fees, and an unconditional timeout refund. Small final payouts and remainders terminate cleanly. | Payout/refund destinations, sequences, fees at every level, zero-fee progress, malformed Taproot scripts/keys, wrong network, invalid or mixed-unit timeouts, and insufficient funds. It does not detect activation status. |
| [Tic-tac-toe](../sapio-contrib/src/contracts/tic_tac_toe.rs) | Current-player signatures authorize moves; all eight winning lines are recognized. Draws split the balance, with the odd satoshi assigned to O, and the other player can claim a 144-block timeout. The shared recursive cache was removed. | All winning lines for both players, a false-diagonal regression, terminal draw outputs, last-move authentication, timeout, independent compilations at different balances, illegal state, and duplicate move keys. Late-game states are compiled in tests; full-game compilation cost is not benchmarked. |
| [Tree payment](../sapio-contrib/src/contracts/treepay.rs) | Recursive payment batching with a maximum fanout and strictly shrinking groups. Radix zero/one, empty payments and zero-valued leaves are rejected. | An uneven 11-recipient tree, every leaf value, total conservation, fanout at every level, invalid input, and underfunding. |
| [Undo send](../sapio-contrib/src/contracts/undo_send.rs) | Immediate return to the sender or delayed forwarding to the recipient. | Both exact destinations and amounts, forward sequence, and underfunding. Undo remains available after the delay until the output is spent; the timeout enables forwarding rather than disabling undo. |
| [Vaults](../sapio-contrib/src/contracts/vault.rs) | Repeated withdrawals, address-backed vaults, and bounded-fanout vault trees. Step counts and amounts must make progress; the final partial branch receives its actual remainder. | Multi-step compilation, cold/hot branches, a 1,500-satoshi tree split into 1,000 and 500, invalid step count/cap/radix, checked multiplication, and underfunding. |

## Derivative examples

These build predetermined settlements and guard them with supplied oracle keys.
The caller must authenticate the oracle and its interpretation of each symbol,
price interval or outcome. This repository does not provide oracle networking or
an attestation service.

| Family | Supported behavior and repairs | Behavioral coverage and assumptions |
| --- | --- | --- |
| [Generic bet and oracle adapters](../sapio-contrib/src/contracts/derivatives/mod.rs) | A finite ordered settlement tree with cooperative exit. Outcomes must be unique, nonempty, single-input settlements with equal collateral. `ThresholdOracle::new` rejects invalid quorums. | Singleton and multiple outcomes, cooperative guard, malformed settlement sets, duplicate endpoints, and quorum validation. The first outcome extends below its first price endpoint. [Oracle APIs](../sapio-contrib/src/contracts/derivatives/oracle.rs) and [contract APIs](../sapio-contrib/src/contracts/derivatives/apis.rs) define the interfaces. |
| [Call](../sapio-contrib/src/contracts/derivatives/call.rs) | Capped call payouts use checked integer scaling and include an off-grid cap. Collateral is `(cap - strike) * notional / PRICE_UNIT`. | Long/short payouts, conservation, grid endpoints and malformed schedules. `PRICE_UNIT` is 10,000; notional is satoshis per oracle-price unit. |
| [Put](../sapio-contrib/src/contracts/derivatives/put.rs) | Put collateral is `strike * notional / PRICE_UNIT`; zero-valued settlement outputs are omitted. | Exact allocations at and around strike, endpoint coverage, conservation, and invalid schedules. The same explicit price-unit convention applies. |
| [Risk reversal](../sapio-contrib/src/contracts/derivatives/risk_reversal.rs) | Finite two-sided payoff schedule with checked wide-integer range/notional arithmetic. | Endpoint payouts, conserved collateral, range/divisor validation, and overflow. |
| [Exploding option](../sapio-contrib/src/contracts/derivatives/exploding.rs) | Funded and partially funded exercise paths with an expiry refund. Prepared `GenericBet` schedules compose with either wrapper. Exercise collateral and added participant funding are checked separately. | Exercise/refund destinations and amounts, collateral errors, and the distinction between a 1,000-satoshi input-zero requirement and 3,000-satoshi aggregate exercise funding. |
| [PowSwap](../sapio-contrib/src/contracts/derivatives/powswap.rs) | Two equal-collateral settlements with positive payments and a cooperative two-key exit. Repeated constraints of one timelock kind use their maximum. | Settlement values, compiled locktimes/sequences, distinct keys, and invalid constraints. Combined locks support relative-height plus absolute-time or relative-time plus absolute-height; incompatible combinations fail. |
| [Signature-attested outcomes](../sapio-contrib/src/contracts/derivatives/signature_attested.rs) | `SignatureAttested` and `OutcomeOracle` use ordinary transaction signatures for selected outcomes, replacing the misleading private DLC sketch. Integer weights conserve all satoshis; equal payout transactions retain every oracle alternative. | Linear/geometric/logistic endpoints, logistic midpoint, rounding, quorum/key guards, duplicate keys, malformed weights, and equal-payout outcomes. Convenience curves quantize shares to one billionth. This is not an adaptor-signature DLC implementation. |

## WASM modules and shared interfaces

The [catalog](../contrib/vectors/examples/catalog.json) contains an input fixture
for every WASM module. The [driver](../contrib/check_examples.py) compares that
inventory with Cargo metadata, so adding a module requires adding a fixture.
Each module runs through the real host with input/output schema validation,
artifact validation where it returns a contract, repeatability across fresh
instances, and rejection of null contract arguments. The catalog also checks
selected output values, transition counts, continuation counts and timelocks.
It loads dependency modules for cross-module examples instead of substituting
native mocks.

| WASM module | Supported behavior and focused checks |
| --- | --- |
| [treepay](../plugin-example/treepay) | Payment batching with checked amount/fee totals and terminating grouping. Native regressions exercise empty payments, invalid radix, fanout and overflow; the catalog checks the two output amounts. |
| [trampolinepay](../plugin-example/trampolinepay) | Delegates batching to the `treepay` module through the shared interface. The catalog resolves the real child module and checks its returned payout. |
| [vault](../plugin-example/vault) | Exposes the library vault constructors. The catalog compiles both root transitions; library tests cover progression, branch values and invalid parameters. |
| [staker](../plugin-example/staker) | Exposes the staked-signer contract. The catalog checks the two root transitions; library tests exercise the keys, burn path and closing delay. |
| [coin_pool](../plugin-example/coin_pool) | Exposes the library pool and its update interface. The catalog compiles its exit; library tests cover refund alignment, odd splits and external funding. |
| [fedpeg](../plugin-example/fedpeg) | Exposes normal and recovery federation parameters. The catalog compiles recovery; library tests check both quorum domains and the recovery delay. |
| [helloworld](../plugin-example/helloworld) | Cooperative two-key escrow with delayed predetermined payouts. The catalog checks both exact escrow output values. |
| [op_return_chain](../plugin-example/op_return_chain) | Exposes signed data/continuation updates. The catalog checks the default suggested transaction; library tests check data limits, signer and fee accounting. |
| [hanukkiah](../plugin-example/hanukkiah) | Exposes the parallel eight-night candle layout, taking one whitespace-separated string of 36 addresses. The catalog compiles the schedule; library tests cover both library layouts and every candle/fee. |
| [payment_pool](../plugin-example/payment_pool) | A separate pool with participant exits and cooperative updates. Signed requests commit to the sender, sequence, fee and payouts; keep `sig_needed` enabled outside debugging. Native tests mutate signatures/requests and check withdrawals, fees and singleton exits; the catalog checks two exit values. |
| [jamesob-vault](../plugin-example/jamesob-vault) | Triggered withdrawal and recovery with optional CPFP outputs. Fee rates are satoshis per 1,000 weight units. Fees round up from exact unsigned sizes; witness growth is excluded. Native tests check rounding, overflow and actual vault funding; the catalog checks both transitions. |
| [nft](../plugin-example/nft) | Owner-authorized transfer into a selected sale module. Mint metadata and royalty parameters are validated; artist blessings are metadata, not verified provenance. The catalog checks the default hold state and exposed sale continuation. |
| [nft-sale](../plugin-example/nft-sale) | A fixed-price, height-locked sale delegates minting of the new owner's NFT. Buyer funds cover seller and artist payouts without inflating the NFT's own amount. The real-module catalog checks all three output values and sale height. |
| [nft-auction](../plugin-example/nft-auction) | An owner-signed Dutch-auction schedule delegates minting to `nft`, with 1–720 decreases and a positive block period. Native regressions check exact price endpoints, height progression and every constructor; the catalog checks all three scheduled heights. |
| [clause-module](../plugin-example/clause-module) | Returns a two-key policy clause. The catalog asserts its exact serialization. |
| [clause-module-trampoline](../plugin-example/clause-module-trampoline) | Obtains that clause from the real `clause-module` child and returns the same policy. The catalog asserts the exact result. |
| [ordinal-example](../plugin-example/ordinal-example) | Owner-authorized ordinal sales with direct and planner-based construction. Native regressions check actual ordinal positions, payout destinations, fees, malformed ranges and auxiliary funding; the catalog checks both exposed continuations. |
| [ordinal-inscription](../plugin-example/ordinal-inscription) | Constructs and carries an inscription through a signed continuation. Native regressions check envelope/payload preservation, ownership and funding; the catalog compiles the artifact and checks its fee-adjusted output. |
| [custom-policy](../plugin-example/custom-policy) | Implements `PolicyCompiler` with an arithmetic signature predicate outside Miniscript. The catalog validates the raw Taproot artifact and its committed payout; native tests check the payment and reject underfunding. |

The two remaining workspace members are interfaces, not WASM entry points:

| Interface | Purpose and assumptions |
| --- | --- |
| [batching-trait](../plugin-example/batching-trait) | Versioned payment-batching request and typed module handle used by `treepay` and `trampolinepay`. Payment amounts remain BTC JSON fields; fee rate is satoshis per byte. |
| [nft-trait](../plugin-example/nft-trait) | Versioned mint/sale requests and typed handles. Royalty fractions must be finite and within zero to one; metadata version and edition bounds are validated. Royalties are quantized to millionths, then rounded down to satoshis. Native tests cover these bounds and exact payout arithmetic. |

The ordinal examples require the caller to supply accurate ordered input ranges;
there is no ordinal indexer here. The planner checks nonempty, nonoverlapping
ranges, complete funding, requested ordinal positions and output padding. It
preserves transaction input order, including disjoint ranges. Each requested
ordinal starts a 501-satoshi output under the example's current padding rule.
That constant is an example policy, not a promise of universal dust acceptance.
The planner greedily fits ordinary payouts into gaps; some otherwise feasible
packings can be rejected. Unknown auxiliary inputs are added only after all
known input sats have been allocated and must cover fees when present.

## Native executables

| Example | Behavior and regression coverage |
| --- | --- |
| [payment](../sapio/examples/payment.rs) | Compiles a fixed 1,000-satoshi payment with a 500-satoshi fee reserve. The test checks the destination, value, one-input shape, 1,500-satoshi input requirement and underfunding. |
| [dcf_mining_pool](../examples/dcf_mining_pool) | Compiles an offline mining reward payout tree from JSON or a deterministic demonstration. Sorted unique keys receive equal shares after every tree transaction's fee is reserved; remainder sats go to the first keys. Tests traverse all levels, check each P2TR recipient exactly once, conserve the full reward and fees, enforce fanout, and reject invalid radix, duplicates, overflow and insufficient rewards. It replaces the unfinished RPC coordinator; share verification, networking and coinbase coordination are not implemented. |

The [custom policy vector exporter](../sapio/examples/custom_policy_vectors.rs)
is also an executable test fixture. Its [isolated node driver](../contrib/check_custom_policy.py)
funds actual regtest outputs and checks custom witnesses against Bitcoin Core.
See [policy backend validation](POLICY_BACKENDS.md#node-validation).

See the [native example instructions](../examples/README.md) for commands and
the mining request format.

## Running the checks

From the repository root, use the pinned toolchain and the LLVM WebAssembly
setup described in [DEVELOPMENT.md](DEVELOPMENT.md):

```sh
cargo test --locked -p sapio-contrib
cargo test --locked -p sapio-base
cargo test --locked -p sapio --lib ordinals
cargo test --locked -p sapio --test artifact_validation --test funding_validation --test ordinal_allocation
cargo test --locked -p sapio --example payment
cargo test --locked -p dcf_mining_pool
bash contrib/sapio_wasm.sh
```

The last script runs native plugin tests, builds the WASM workspace and host,
and executes both the complete catalog and the focused CLI checks. These checks
establish the tested artifact, amount, authorization and schema properties;
they do not broadcast transactions or exercise every possible witness on a
Bitcoin node.
