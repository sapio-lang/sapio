# Native examples

The payment and mining examples compile deterministic Regtest artifacts using
native CTV research semantics. They run offline and do not fund or broadcast
transactions.

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

## Purrfect Vault transaction flow

The integration workspace also supplies an offline Purrfect Vault example:

```sh
cargo run --locked -p sapio_integration_tests --example purrfect_vault
```

It compiles and signs a shared-address trigger, cancellation, another trigger
and a withdrawal with a native twenty-block CSV guard. The full principal is
preserved; a separate sponsor funds the 546-satoshi destination marker and
fees. A pinned WASM-v2 oracle executes the CAT covenant Script, while
the owner signs every operation natively.

To fund and confirm that flow on an isolated regtest node:

```sh
cargo build --locked -p sapio_integration_tests --example purrfect_vault_vectors
python3 contrib/check_purrfect_vault.py /path/to/bitcoind /path/to/bitcoin-cli \
  target/debug/examples/purrfect_vault_vectors
```

The driver checks cancellation, the exact CSV boundary, fees, payout and marker
outputs, and invalid signed witnesses. See [Purrfect Vault](../docs/PURRFECT_VAULT.md)
for the pinned upstream reference, supported profile and oracle assumptions.

## LayerTree MID transaction flow

Run the signed dense LayerTree lifecycle offline:

```sh
cargo run --locked -p sapio_integration_tests --example layertree
```

The example uses disposable keys and synthetic coins to join and graft members,
rewrite a cohort, update a member, split balances, redeem vouchers, exit, and
evict expired cohorts. A separate input funds admissions and fees. Each exact
MID Script executes in a bounded WASM interpreter before the oracle signs its
native Taproot wrapper. The state root changes the physical internal key while
the complete script-tree commitment remains stable.

To fund and confirm the lifecycle on an isolated regtest node:

```sh
cargo build --locked -p sapio_integration_tests --example layertree_vectors
python3 contrib/check_layertree.py /path/to/bitcoind /path/to/bitcoin-cli \
  target/debug/examples/layertree_vectors
```

The [LayerTree documentation](../docs/LAYERTREE.md) specifies the eleven-branch
profile, upstream vectors, funding and proof requirements, and oracle trust.
It also explains the terminal empty-tree transaction and the wallet and
Lightning components outside this implementation.
