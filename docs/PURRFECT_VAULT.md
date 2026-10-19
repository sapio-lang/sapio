# Purrfect Vault with executed CAT Script

The example executes Bitcoin Script with OP_CAT in a bounded WASM interpreter.
Its immutable program parameters contain the actual Script; the interpreter
has no vault-stage, principal, or destination predicate implemented in Rust.

```sh
cargo run --locked -p sapio_integration_tests --example purrfect_vault
cargo test --locked -p sapio_integration_tests --test purrfect_vault
```

The offline runner compiles typed actions, binds synthetic coins, constructs
Script witnesses, obtains oracle and native signatures, and finalizes a
trigger → cancel → trigger → complete sequence. The [node driver](../contrib/check_purrfect_vault.py)
funds and confirms the same flow on an isolated regtest node.

## One address, three spending paths

The reference is Taproot Wizards’ MIT-licensed
[source at 9e41e4d](https://github.com/taproot-wizards/purrfect_vault/tree/9e41e4dc0a7dc5b6267d117eb8b60cbf594e02ff).
The [ported Script](../sapio-contrib/src/contracts/purrfect_vault/script.rs)
retains its copyright and license. That revision requires the same owner key
on every path; its README predates the signature requirement.

Every operation spends the vault at input zero and a native witness-v0 sponsor
at input one. Let `V` be the vault, `A` its principal, and `D` a destination.

| Operation | Outputs | Native guard |
| --- | --- | --- |
| Trigger | `A` to `V`, then 546 sats to `D` | Owner signature |
| Cancel | `A` to `V` | Owner signature |
| Complete | `A` to the previous trigger’s `D` | Owner signature and CSV20 |

Deposit, triggered and cancelled principal outputs use one identical address.
Retriggering may change the destination and restart the delay. Cancel may spend
any vault output; its single-output parent cannot complete a withdrawal. The
marker is an ordinary spendable output containing `D` directly. Spending it
does not invalidate completion. Cancellation returns to the same key’s vault;
there is no separate cold recovery key.

## Executed predicate and canonical fields

[`PurrfectVault`](../sapio-contrib/src/contracts/purrfect_vault.rs) exposes
`trigger(context, destination)`, `cancel(context)`, and
`complete(context, previous_trigger)`. Proposals, principal and local fee caps
do not change the funded address.

Script reconstructs BIP341 commitments using CAT and SHA256, then validates a
synthetic generator-key Schnorr signature. Complete also reconstructs the
parent transaction with HASH256 and binds its output zero and destination.
The [witness constructor](../sapio-contrib/src/contracts/purrfect_vault/witness.rs)
supplies evidence and a genuine owner signature; the guest independently checks
both signatures against the authenticated native wrapper leaf.

The port fixes ambiguous witness boundaries in the reference scripts. Script
requires exact widths for the 63/1/1 signature split, eight-byte amounts,
serialized P2TR vault script, fixed BIP341 fields and parent transaction fields.
Completion constructs the parent’s two-input count and empty scriptSigs itself.
These checks live in committed Script, rather than vault-specific host code.

Destinations contain **1 through 127 raw Script bytes**. Script derives their
canonical one-byte CompactSize prefix from OP_SIZE, preventing evidence from
encoding additional outputs. This includes standard address scripts. The size
profile and canonical field checks are deliberate upstream differences; parent
reconstruction remains within CAT’s 520-byte element limit.

The synthetic signature increments its challenge’s last byte. Binding grinds
the sponsor’s BIP68-disabled sequence before any signatures exist. The saved
artifact retains the resulting proposal, template commitment and fee rules,
so reloaded spend intents still enforce the same local funding constraints.

## Enforcement, fees and checks

See the [Script interpreter profile](BITCOIN_SCRIPT_CAT.md) for supported opcodes
and resource bounds. CAT stays inside WASM; the native wrapper uses ordinary
Taproot signatures and independently enforces the owner key and CSV20. The
oracle must remain honest and available. An owner and dishonest oracle can
bypass emulated conditions together. Bitcoin checks native signatures, Taproot
proofs and chain maturity; native CAT activation is not assumed.

The principal is preserved. The sponsor pays the marker and remaining fee on
trigger; cancel and complete consume the sponsor as fees, without change. The
fixture deposits 100,000 sats and uses four independent 2,000-sat P2WPKH sponsors:
each trigger pays 1,454 sats in fees; cancel and complete pay 2,000. Both markers
remain spendable after the final withdrawal.

```sh
cargo test --locked -p sapio-contrib purrfect_vault
cargo test --locked -p sapio_integration_tests --test bitcoin_script_cat --test purrfect_vault
cargo test --locked --manifest-path evaluators/Cargo.toml -p sapio-bitcoin-script-cat-evaluator
bash evaluators/build.sh --check
cargo build --locked -p sapio_integration_tests --example purrfect_vault_vectors
python3 contrib/check_purrfect_vault.py /path/to/bitcoind /path/to/bitcoin-cli \
  target/debug/examples/purrfect_vault_vectors
```

Tests execute the checked-in WASM, compare sighashes with rust-bitcoin, reload
artifacts and intents, and check signatures, field boundaries, principal,
destinations, parent linkage and CSV. The node driver checks cancellation, fees,
markers and invalid signatures. Completion fails at tip `H+18`, is admitted for
the next block at `H+19`, and confirms at `H+20`. CI uses Bitcoin Core 31.1.
