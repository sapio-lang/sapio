# Rust Lib/Bin

There's not much to be said here. Sapio code is just Rust code, so it can be
shipped as a standalone rust library or binary tool.

This code can then be integrated into any codebase either natively or using
FFI.

It's a good idea to always package contracts as a library separate from the
binary, so that if a user wants to natively incorporate the contract it is
easy to do, and the packaged WASM or binary can be a utility based on it.

