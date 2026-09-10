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

The v1 programs and `common.rs` retain their original ABI and bytes. V2 programs
use `v2.rs` for invocation plumbing and the [fragment SDK](fragments/README.md)
for typed operations. There is no interpreter in these libraries for Bitcoin
Script or arbitrary host code.

The fragment SDK's pure decoder/encoding tests can run natively:

```sh
cargo test --locked --manifest-path evaluators/Cargo.toml -p sapio-covenant-fragments --lib
```

Actual cryptographic and predicate checks execute the compiled modules through
the program oracle's tests in the parent workspace.
