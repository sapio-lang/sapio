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
1.  Clone this repo: 
```
git clone git@github.com:sapio-lang/sapio.git && cd sapio
```
1.  \[Optional\] To use dependencies from [crates.io](https://crates.io)
```
git checkout v0.1.4 && cp plugin-example .. && cd ..
```
1.  Build the plugin
```
cd plugin-example && wasm-pack build && cd ..
```
1.  Instantiate a contract from the plugin:
```
cargo run --bin sapio-cli -- contract create 9.99 "{\"participants\": [{\"amount\": 9.99, \"address\": \"bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw\"}], \"radix\": 2}" --file="plugin-example/pkg/sapio_wasm_plugin_example_bg.wasm"
```

You can use `cargo run --bin sapio-cli -- help` to learn more about what a the CLI
can do! and `cargo run --bin sapio-cli -- <subcommand> help` to learn about
subcommands like `contract`.


## Docs

You can review the docs either by building them locally or viewing
[online](https://docs.rs/sapio).
