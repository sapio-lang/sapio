# Installing Sapio

## QuickStart:

Sapio should work on all platforms, but is recommend for use with Linux (Ubuntu preferred).
Follow this quickstart guide to get going.

1.  Get [rust](https://rustup.rs/) if you don't have it already.
1.  Add the wasm target and nightly toolchain by running the below command in your terminal:
```bash
rustup update
```
1.  Get the [wasm-pack](https://rustwasm.github.io/wasm-pack/) tool.

> Tip: On macOS you may need to do the following:
> ```bash
> brew install llvm
> cargo install wasm-pack
> ```
> and then load the following before compiling
> ```bash
> export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
> # for older homebrew installs
> # export PATH="/usr/local/opt/llvm/bin:$PATH"
> export CC=/opt/homebrew/opt/llvm/bin/clang
> export AR=/opt/homebrew/opt/llvm/bin/llvm-ar
> ```

1.  Clone this repo: 
```
git clone --depth 1 git@github.com:sapio-lang/sapio.git && cd sapio
```
We recommend a shallow clone unless you want the full history.
1.  Build a plugin
```
cd plugin-example/treepay/ && wasm-pack build && cd ..
```
1.  Instantiate a contract from the plugin:
```
cargo run --bin sapio-cli -- contract create \{\"amount\":9.99,\"arguments\":\{\"Treepay\":\{\"fee_sats_per_tx\":1000,\"participants\":\[\{\"address\":\"bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw\",\"amount\":2.99\}\],\"radix\":2\}\},\"network\":\"Regtest\"\} --file="plugin-example/treepay/pkg/sapio_wasm_plugin_example_bg.wasm"
```

You can use `cargo run --bin sapio-cli -- help` to learn more about what a the
CLI can do! and `cargo run --bin sapio-cli -- <subcommand> help` to learn about
subcommands like `contract`. If you aren't modifying Sapio itself, you'll want
to run `cargo build --release` and use a release binary as it is much faster.
1. Install Sapio Studio

[Sapio Studio](https://github.com/sapio-lang/sapio-studio) is an in-development
graphical user interface for Sapio. It is the recommended way to get started with Sapio development.
We recommend a shallow clone unless you want the full history.
```
git clone --depth 1 git@github.com:sapio-lang.sapio-studio.git && cd sapio-studio
yarn install
```
and then in separate shells
```
yarn start-react
yarn start-electron
```

The first time you run it you may have some errors, you will need to ensure
you've configured your client correctly.


## Docs

You can review the docs either by building them locally or viewing
[online](https://docs.rs/sapio).
