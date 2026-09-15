# Shared TreePay contract

The reusable Rust implementation and checked constructors behind the TreePay
WASM module. This crate has no plugin registration or WASM entry point.

Its native tests check payment and fee preservation, invalid tree shapes,
underfunding and arithmetic overflow. Run them with:

```sh
cargo test --locked --manifest-path plugin-example/Cargo.toml -p sapio-treepay-contract
```
