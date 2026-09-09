# Sapio WASM examples

This workspace contains **19 executable guests and two interface libraries**.
The examples are maintained demonstrations of contract construction. They do not
include a production wallet, a deployed federation, or audited financial protocols.

## Build and exercise the catalog

From the repository root, use the pinned Rust toolchain and LLVM Clang with
WebAssembly support. On macOS, set `CC_wasm32_unknown_unknown` to the LLVM Clang
binary when the system compiler lacks that target.
See the [development guide](../docs/DEVELOPMENT.md) for platform setup.

```sh
CARGO_INCREMENTAL=0 bash contrib/sapio_wasm.sh
```

The script runs native plugin tests, builds every guest, executes the real host
catalog, and runs the CLI cross-module and inscription smoke checks. The guest compiler runs inside metered WASM, so executable guests use optimized
release builds with Cargo; its artifact is in
`plugin-example/target/wasm32-unknown-unknown/release/`, not a `wasm-pack` package.

For native tests alone:

```sh
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path plugin-example/Cargo.toml --workspace
```

[Catalog inputs](../contrib/vectors/examples/catalog.json) provide a complete
`CreateArgs` example for every executable guest. Each has an `arguments` payload
and a `context` with the network, integer satoshi amount, optional effects, and
optional ordinal ranges. Individual argument fields explicitly use sats, BTC,
or the tagged `CoinAmount` form; consult that guest's generated API schema.
The runner replaces `MODULE_<directory>` placeholders with actual loaded module
hashes. These placeholders are fixture syntax, not values accepted by a guest.

Each catalog case checks a real guest's advertised schema, compiles its valid
input, verifies the expected artifact or clause, repeats from a fresh instance,
and rejects null arguments. Focused native tests cover arithmetic and
continuation behavior that a default compilation does not exercise. Successful
compilation does not broadcast or independently establish every transaction's
network policy acceptability.

## Executable guests

| Directory | Purpose and assumptions | Focused coverage |
| --- | --- | --- |
| [treepay](treepay/) | **Tree payments.** Builds a bounded-radix tree with a fixed fee per transaction. Recipients and amounts must be nonempty and positive; radix must be at least two. | Tree shape, payment totals, fees, invalid radix and overflow. |
| [trampolinepay](trampolinepay/) | **Delegated tree payments.** Calls a batching module and funds the returned contract. The selected module must implement a compatible batching wire interface. | Catalog calls the actual treepay guest. |
| [vault](vault/) | **Staged vault.** Wraps the library vault with address or tree cold-storage destinations. Timelocks, step counts, payouts and tree limits are validated by the library. | Catalog plus library vault tests. |
| [staker](staker/) | **Bonded signer.** Provides redemption and burn paths for a signing key. This example does not implement a complete staking protocol. | Catalog plus library staker tests. |
| [coin_pool](coin_pool/) | **Coin pool.** Splits a pool into participant refunds and exposes cooperative updates. The guest exposes the Basic key-and-amount interface. | Catalog plus library coin-pool tests. |
| [fedpeg](fedpeg/) | **Federated peg.** Provides normal and delayed recovery threshold paths. Keys and thresholds are contract parameters; external chain verification is outside this example. | Catalog plus library federation tests. |
| [helloworld](helloworld/) | **Escrow.** Provides a cooperative signature path and delayed payouts to two escrow addresses. | Catalog checks both escrow output amounts. |
| [op_return_chain](op_return_chain/) | **Data chain.** Appends OP_RETURN data or closes to its owner. Continuation fees come from the available balance. | Catalog plus library continuation tests. |
| [hanukkiah](hanukkiah/) | **Scheduled candles.** Builds eight timed transactions paying 36 candles. Its recipient field is one whitespace-separated string containing 36 addresses. | Catalog plus library schedule and recipient tests. |
| [payment_pool](payment_pool/) | **Signed payment pool.** Authenticates payment requests by sequence, fees, payouts and sender; records fees and gives a final member a direct exit. Balance totals must equal the available funds. Keep sig_needed true outside debugging. | Native signature mutation, overspending, ejection, fee and withdrawal tests. |
| [jamesob-vault](jamesob-vault/) | **Vault with recovery.** Provides hot/cold spending, delayed redemption and optional CPFP outputs. The fee rate is sats per 1000 weight units, using exact unsigned sizes that exclude witness growth; fees round up. | Native overflow, rounding and compiled-funding tests. |
| [nft](nft/) | **Metadata NFT.** Commits metadata and lets the owner propose a sale through a selected module. Metadata requires version zero and a valid one-based edition. Artist blessings are committed metadata, not verified provenance. | Catalog compiles the real minting guest. |
| [nft-sale](nft-sale/) | **Fixed NFT sale.** Transfers the NFT after the sale height and splits the buyer price between owner and artist. Extra buyer funds are a separate input; the NFT value is preserved. | Catalog checks reminting, royalty outputs and lock time. |
| [nft-auction](nft-auction/) | **Dutch NFT auction.** Provides owner-signed sales from the starting price through the exact minimum. Schedules allow 1–720 decreases and a positive block period; the default spans 4,320 blocks at six-block intervals. | Native endpoint, overflow and all-constructor validation; catalog checks three prices. |
| [clause-module](clause-module/) | **Clause producer.** Returns a clause requiring both supplied keys. Its result is a clause string rather than a compiled contract. | Catalog checks the exact clause. |
| [clause-module-trampoline](clause-module-trampoline/) | **Delegated clause producer.** Calls another clause module through the typed handle. Actual host calls validate advertised schemas. | Catalog calls the actual clause producer. |
| [ordinal-example](ordinal-example/) | **Ordinal sale.** Preserves the requested sat at the start of a 501-sat buyer output, with direct and planned sale continuations. Ordered input ranges must be complete, non-overlapping and include the target plus padding. | Native target-position, payment, fee, missing-target and planner tests. |
| [ordinal-inscription](ordinal-inscription/) | **Inscription reveal.** Commits an Ord envelope under the owner signature and reveals to the owner or a selected address. Complete non-overlapping ranges and enough funds for the inscribed sat, padding and fee are required. | Native signed PSBT/artifact round trip, body chunking and invalid-input tests; CLI smoke. |
| [custom-policy](custom-policy/) | **Custom policy backend.** Implements a checked arithmetic signature predicate composed with a template covenant through `PolicyCompiler`. Witness construction remains the backend's responsibility. | Native checked payment and underfunding rejection; real WASM catalog checks the raw Taproot artifact and payout. |

## Interface libraries

- [`batching-trait`](batching-trait/) defines versioned payments and batching
  handles shared by TreePay and TrampolinePay.
- [`nft-trait`](nft-trait/) defines versioned mint/sale payloads and the sell
  continuation. Royalties must be finite fractions from zero to one. They are
  rounded to millionths, then each payout is rounded down to whole satoshis using
  integer arithmetic. The shared royalty code has boundary and overflow tests.

These two crates produce Rust libraries, not executable guest modules. Their
versioned JSON enum names remain the wire interface. Resolving a typed handle
only resolves a module identity; the host checks actual arguments and successful
results against the receiver's schema on every call.

The former `dao` directory contained only a manifest pointing at missing source.
That placeholder has been removed; this workspace does not provide a DAO module.

## Funding and ordinal boundaries

CTV templates distinguish total transaction funding from the amount required at
the contract's input. An added buyer payment requires an auxiliary transaction
input; supplied funding is checked when binding. Examples that use fixed fees
are demonstrations, not a live fee estimator. Zero-fee catalog fixtures are not
claims about mempool relay policy.

Ordinal examples accept half-open ranges in transaction input order. They first
allocate all known input sats, then add any externally funded inputs with unknown
ordinal ranges. They retain a fixed 500-sat padding convention; this is an example
policy, not an Ord protocol requirement. The planner is deterministic and greedy,
so it can reject a payout arrangement for which another packing might exist.

The host's execution, memory and nested-call limits still apply to all examples.
Large trees, schedules or metadata can hit those limits; a resource rejection is
not evidence that a contract's transactions would be invalid on Bitcoin.
