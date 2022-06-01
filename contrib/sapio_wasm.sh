#!/bin/sh -ex

CC=clang-9 cargo build --release --target wasm32-unknown-unknown --manifest-path plugin-example/Cargo.toml

CLAUSE="$(cat contrib/vectors/clause_input.json | ./target/release/sapio-cli contract create --file plugin-example/target/wasm32-unknown-unknown/release/sapio_wasm_clause.wasm)"
EXPECTED="$(cat contrib/vectors/clause_output.json)"
if [ "$CLAUSE" = "$EXPECTED" ]; then
    echo "Good"
else
    exit 1
fi
