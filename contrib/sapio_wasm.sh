#!/bin/sh -ex

cargo build --release
cargo build --release --target wasm32-unknown-unknown --manifest-path plugin-example/Cargo.toml

CLAUSE="$(cat contrib/vectors/clause_input.json | ./target/release/sapio-cli --config contrib/vectors/basic_config.json contract create --file plugin-example/target/wasm32-unknown-unknown/release/sapio_wasm_clause.wasm | jq '.result.Ok.Call.result')"
EXPECTED="$(cat contrib/vectors/clause_output.json)"
if [ "$CLAUSE" = "$EXPECTED" ]; then
    echo "Clause Compilation Good"
else
    echo "Failed"
    exit 1
fi

CLAUSE_KEY="$(./target/release/sapio-cli --config contrib/vectors/basic_config.json contract load --file plugin-example/target/wasm32-unknown-unknown/release/sapio_wasm_clause.wasm | jq '.result.Ok.Load.key' | xargs echo)"
TRAMPOLINED="$(cat contrib/vectors/trampoline_clause_input.json| sed s,TEMPLATE_ARG_A,$CLAUSE_KEY, | ./target/release/sapio-cli --config contrib/vectors/basic_config.json contract create --file plugin-example/target/wasm32-unknown-unknown/release/sapio_wasm_clause_trampoline.wasm | jq '.result.Ok.Call.result')"

if [ "$TRAMPOLINED" = "$EXPECTED" ]; then
    echo "Value Good Through Trampoline"
else
    echo "Failed"
    exit 1
fi
