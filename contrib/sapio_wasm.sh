#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# LLVM Clang with the WebAssembly target is required by secp256k1's C code.
# On macOS set CC_wasm32_unknown_unknown to Homebrew LLVM's clang.
cargo test --locked --manifest-path plugin-example/Cargo.toml --workspace
cargo build --locked -p sapio-cli
cargo build --locked -p sapio-wasm-plugin --features host --example check_examples
# Contract compilation runs inside the guest. Optimize it before applying fuel
# limits; debug compiler guests can also approach the 128 MiB source-size cap.
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path plugin-example/Cargo.toml
python3 contrib/check_examples.py target/debug/examples/check_examples plugin-example/target/wasm32-unknown-unknown/release
python3 contrib/check_wasm.py target/debug/sapio-cli plugin-example/target/wasm32-unknown-unknown/release
