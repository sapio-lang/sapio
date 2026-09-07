# Sapio WASM examples

This separate workspace builds Rust contracts as WASM modules. From the repository
root, run:

```sh
bash contrib/sapio_wasm.sh
```

Use the pinned Rust toolchain and LLVM Clang with the WebAssembly target. See the
[development guide](../docs/DEVELOPMENT.md) for macOS/Linux setup and focused
commands. The script builds all examples and checks direct and cross-module
compilation through the CLI. Zig and wasm-pack are not required.

Examples are research material unless identified as supported in the
[modernization plan](../docs/MODERNIZATION.md). Successful compilation does not
establish safe funding, signer assumptions, or chain support for CTV.
