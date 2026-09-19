# Installing Sapio

Use Rust with rustup, Git and a native C compiler. The maintained developer
preview starts from one source checkout and generates an independent project:

```sh
git clone https://github.com/sapio-lang/sapio.git
cd sapio
cargo build --locked -p sapio-cli
export PATH="$PWD/target/debug:$PATH"
sapio-cli new ../my-contract --name my-contract
cd ../my-contract
cargo test --locked
```

Rustup selects the checked-in stable toolchain. The project includes its full
dependency pins, Cargo lockfile and exact payment evaluator bytes. Cargo fetches
the required repositories; no manually cloned dependency libraries, nightly
toolchain, wasm-pack, container or graphical application are required.

If you use `CARGO_TARGET_DIR`, put its `debug` subdirectory on `PATH` instead.
The [maintained quickstart](https://github.com/sapio-lang/sapio/blob/master/docs/QUICKSTART.md)
and generated `README.md` continue through artifact inspection and a completed
synthetic spend. The example has public demonstration keys and must not receive
real funds.

Building compiler plugins as WASM is a separate workflow. It additionally needs
LLVM Clang with the `wasm32` target; see the
[development guide](https://github.com/sapio-lang/sapio/blob/master/docs/DEVELOPMENT.md).
The starter does not need to rebuild its included evaluator.
