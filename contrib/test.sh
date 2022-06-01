#!/bin/sh -ex
# from https://github.com/rust-bitcoin/bitcoin_hashes/blob/master/contrib/test.sh


# Use toolchain if explicitly specified
if [ -n "$TOOLCHAIN" ]
then
    alias cargo="cargo +$TOOLCHAIN"
fi

cargo --version
rustc --version

# Make all cargo invocations verbose
export CARGO_TERM_VERBOSE=true

# Defaults / sanity checks
cargo build --release --all
cargo test --release --all

if [ "$DO_FEATURE_MATRIX" = true ]; then
    cargo build --all
    cargo test --all
fi

# Docs
if [ "$DO_DOCS" = true ]; then
    cargo doc --all
fi


# Address Sanitizer
if [ "$DO_ASAN" = true ]; then
    cargo clean
    CC='clang -fsanitize=address -fno-omit-frame-pointer'                                        \
    RUSTFLAGS='-Zsanitizer=address -Clinker=clang -Cforce-frame-pointers=yes'                    \
    ASAN_OPTIONS='detect_leaks=1 detect_invalid_pointer_pairs=1 detect_stack_use_after_return=1' \
    cargo test --lib --all -Zbuild-std --target x86_64-unknown-linux-gnu
    cargo clean
    CC='clang -fsanitize=memory -fno-omit-frame-pointer'                                         \
    RUSTFLAGS='-Zsanitizer=memory -Zsanitizer-memory-track-origins -Cforce-frame-pointers=yes'   \
    cargo test --lib --all -Zbuild-std --target x86_64-unknown-linux-gnu
fi

# Bench
if [ "$DO_BENCH" = true ]; then
    cargo bench --all
fi

if [ "$DO_SAPIO_WASM" = true ]; then
    sh contrib/sapio_wasm.sh
fi

