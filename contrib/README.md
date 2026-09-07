# Development checks

Run these from the repository root:

- `bash contrib/test.sh`: native tests, feature checks, formatting, Clippy and API docs.
- `bash contrib/sapio_wasm.sh`: build WASM examples and run the CLI smoke test.

Both use the pinned toolchain and committed lockfiles. The WASM script requires
LLVM Clang with WebAssembly support and Python 3. See the
[development guide](../docs/DEVELOPMENT.md) for setup.
