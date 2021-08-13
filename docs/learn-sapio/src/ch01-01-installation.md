# Installing Sapio

## QuickStart:

Sapio should work on all platforms, but is recommend for use with Linux (Ubuntu preferred).
Follow this quickstart guide to get going.

1.  Get [rust](https://rustup.rs/) if you don't have it already.
1.  Add the wasm target by running the below command in your terminal:
```bash
rustup target add wasm32-unknown-unknown
```
1.  Get the [wasm-pack](https://rustwasm.github.io/wasm-pack/) tool.

> Tip: On an M1 Mac you may need to:
> ```bash
> brew install llvm
> cargo install wasm-pack
> rustup toolchain install nightly
> ```
> and then load the following before compiling
> ```bash
> export PATH="/opt/homebrew/opt/llvm/bin:$PATH".
> export CC=/opt/homebrew/opt/llvm/bin/clang
> export AR=/opt/homebrew/opt/llvm/bin/llvm-ar
> rustup toolchain default nightly
> ```

1.  Clone this repo: 
```
git clone git@github.com:sapio-lang/sapio.git && cd sapio
```
1.  Build a plugin
```
cd plugin-example/treepay/ && wasm-pack build && cd ..
```
1.  Instantiate a contract from the plugin:
```
cargo run --bin sapio-cli -- contract create \{\"amount\":9.99,\"arguments\":\{\"Basic\":\{\"fee_sats_per_tx\":1000,\"participants\":\[\{\"address\":\"bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw\",\"amount\":2.99\}\],\"radix\":2\}\},\"network\":\"Regtest\"\} --file="plugin-example/treepay/pkg/sapio_wasm_plugin_example_bg.wasm"
```

You can use `cargo run --bin sapio-cli -- help` to learn more about what a the CLI
can do! and `cargo run --bin sapio-cli -- <subcommand> help` to learn about
subcommands like `contract`.


## Docs

You can review the docs either by building them locally or viewing
[online](https://docs.rs/sapio).
