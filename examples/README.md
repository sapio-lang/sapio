# Native examples

Both examples compile deterministic Regtest artifacts using native CTV research
semantics. They run offline and do not fund or broadcast transactions.

```sh
cargo run --locked -p sapio --example payment > payment.json
cargo run --locked -p dcf_mining_pool > mining.json
```

The payment example sends 1,000 satoshis to a fixed destination and reserves
500 satoshis for its transaction fee.

The mining example divides a 50,003-satoshi reward among five demonstration
keys. It reserves 100 satoshis for every transaction in a radix-four payout
tree, then divides the remaining reward evenly. Any remainder goes to the first
keys in sorted order, so changing input order does not change the result. Each
miner receives a standard P2TR output derived from their internal public key.

Supply your own request as a JSON file, or use `-` to read stdin:

```sh
cargo run --locked -p dcf_mining_pool -- request.json
```

```json
{
  "miners": [
    "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
    "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
  ],
  "reward_sats": 10000,
  "radix": 4,
  "fee_sats_per_tx": 100
}
```

Amounts are integer satoshis. Empty or duplicate miner lists, radix below two,
overflow and insufficient rewards fail. The compiler checks every emitted
artifact. It replaces the historical nonfunctional RPC coordinator; it does not
implement share verification, pool networking or a coinbase coordinator.

The native suite tests conservation across all payout-tree levels, every miner's
destination, fee totals, remainders and invalid inputs:

```sh
cargo test --locked -p dcf_mining_pool
cargo test --locked -p sapio --example payment
```

See the [complete contract inventory](../docs/EXAMPLES.md) for library and WASM
examples and their individual assumptions.
