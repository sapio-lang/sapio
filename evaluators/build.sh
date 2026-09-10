#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

mode="${1:---check}"
if [[ "$mode" != --check && "$mode" != --write ]]; then
    echo "usage: $0 [--check|--write]" >&2
    exit 2
fi

# Pin compiler and code generation as part of the program's exact identity.
# Stripping names/debug data removes build-directory-dependent metadata.
export CARGO_INCREMENTAL=0
export CARGO_NET_OFFLINE=true
export CARGO_TARGET_DIR="$PWD/target"
export RUSTFLAGS='-C link-arg=-zstack-size=65536 -C link-arg=--initial-memory=4194304 -C link-arg=--max-memory=4194304'
cargo +1.98.1 build --locked --release --target wasm32-unknown-unknown
for name in ctv pay_at_least; do
    built="target/wasm32-unknown-unknown/release/sapio_${name}_evaluator.wasm"
    artifact="artifacts/${name}.wasm"
    if [[ $(wc -c < "$built") -gt 65536 ]]; then
        echo "$name evaluator exceeds the 64 KiB program limit" >&2
        exit 1
    fi
    if [[ "$mode" == --write ]]; then
        mkdir -p artifacts
        install -m 644 "$built" "$artifact"
    else
        cmp "$built" "$artifact"
    fi
done
