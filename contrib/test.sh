#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo fmt --manifest-path evaluators/Cargo.toml --all -- --check
bash evaluators/build.sh --check
cargo test --manifest-path evaluators/Cargo.toml -p sapio-covenant-fragments --locked
cargo test --workspace --all-features --locked
cargo test -p sapio --example payment --locked
cargo check -p sapio-wasm-plugin --no-default-features --locked
cargo check -p sapio-wasm-plugin --no-default-features --features host --locked
cargo clippy --workspace --all-targets --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
