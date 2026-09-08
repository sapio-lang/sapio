#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# LLVM Clang with the WebAssembly target is required by secp256k1's C code.
# On macOS set CC_wasm32_unknown_unknown to Homebrew LLVM's clang.
cargo test --locked --manifest-path plugin-example/Cargo.toml -p sapio-wasm-ordinal-inscription
cargo build --locked -p sapio-cli
cargo build --locked --target wasm32-unknown-unknown --manifest-path plugin-example/Cargo.toml
python3 contrib/check_wasm.py target/debug/sapio-cli plugin-example/target/wasm32-unknown-unknown/debug
