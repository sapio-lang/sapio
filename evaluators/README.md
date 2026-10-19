# Compiled evaluator programs

This excluded Cargo workspace builds the small programs distributed with Sapio.
They use `no_std`, have no third-party dependencies, and have fixed four-MiB
memories with bounded input arenas and scratch storage.

```sh
bash evaluators/build.sh --check
```

The script pins Rust 1.98.1 and compares the rebuilt bytes with every checked-in
artifact. Use `--write` only when deliberately updating program code. Stripped
names and debug information keep build-directory paths out of the modules.
Program identities commit the actual bytes, so changing an artifact changes
the corresponding program's keys.

| Artifact | ABI | Predicate |
| --- | --- | --- |
| `ctv.wasm` | v1 | Exact BIP119 commitment over native-witness inputs |
| `pay_at_least.wasm` | v1 | Registered interpreter for a fixed minimum payment |
| `templatehash.wasm` | v2 | Exact BIP446 TemplateHash, including the selected annex |
| `template_authorization.wasm` | v2 | TemplateHash plus CSFS under a pinned, physical internal, or proven related key |
| `op_vault.wasm` | v2 | Dynamic vault leaf update and fixed recovery, under the [single-vault BIP345 profile](op-vault/README.md) |
| `depots.wasm` | v2 | Depot resolution: target reveal, guess-signature-bound hits/claims and double-hit burns over CTV+CSFS-style commitments |
| `purrfect_vault.wasm` | v2 | Purrfect Vault trigger/cancel/complete transaction predicates, with authenticated stripped-parent evidence and a separately sponsored destination marker |

The v1 programs and `common.rs` retain their original ABI and bytes. V2 programs
use `v2.rs` for invocation plumbing and the [fragment SDK](fragments/README.md)
for typed operations. There is no interpreter in these libraries for Bitcoin
Script or arbitrary host code.

The fragment SDK's pure decoder/encoding tests can run natively:

```sh
cargo test --locked --manifest-path evaluators/Cargo.toml -p sapio-covenant-fragments --lib
cargo test --locked --manifest-path evaluators/Cargo.toml -p sapio-purrfect-vault-evaluator
```

Actual cryptographic and predicate checks execute the compiled modules through
the program oracle's tests in the parent workspace. The complete OP_VAULT
custody routes run through the oracle in
`contrib/build-a-vault/emulation/tests/custody.rs`.

The [Purrfect Vault example](../docs/PURRFECT_VAULT.md) composes its evaluator
with native owner-signature and twenty-block CSV guards. Its Core driver
confirms cancellation and withdrawal using standard Bitcoin Script witnesses;
the WASM predicate emulates OP_CAT introspection rather than executing OP_CAT.
