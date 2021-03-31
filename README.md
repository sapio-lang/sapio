# Sapio &emsp; [![Build Status]][actions]
[Build Status]: https://github.com/sapio-lang/sapio/workflows/Continuous%20integration/badge.svg
[actions]: https://github.com/sapio-lang/sapio/actions?query=branch%3Amaster
**a framework for creating composable multi-transaction Bitcoin Smart Contracts.**

<img src="https://github.com/sapio-lang/sapio/raw/master/.github/logo.png" alt="Say hi to Jared">



The root crate is a workspace for various Sapio Components such as:

1. [Sapio CLI](cli/): Easy to use interface for using and running sapio contracts.
1. [Sapio Language](sapio/): Base Specification for Sapio Language and Contract Generation
1. [Plugin Example](plugin-example/): Example Project for a Sapio Plugin
1. [Sapio Contrib](sapio-contrib/): Contract modules / functionality made available for general use
1. [Plugin Framework](plugin/): Library for bundling Sapio Plugins
1. [CTV Emulator](ctv_emualtors/): Emulation protocols and servers for CheckTemplateVerify.
1. [Sapio Front](sapio-front/): Protocols for interacting with a compilation session
1. [Sapio Compiler Server](sapio-ws/): Binary for a websocket server running sapio-front

## QuickStart:

Sapio should work on all platforms, but is recommend for use with Linux (Ubuntu preferred).
Follow this quickstart guide to get going.

1.  Get [rust](https://rustup.rs/) if you don't have it already.
1.  Add the wasm target by running `rustup target add wasm32-unknown-unknown` in your terminal.
1.  Get the [wasm-pack](https://rustwasm.github.io/wasm-pack/) tool.
1.  Clone this repo: `git clone git@github.com:sapio-lang/sapio.git && cd sapio`
1.  Build the plugin `cd plugin-example && wasm-pack build && cd ..`
1.  Instantiate a contract from the plugin: `cargo run --bin sapio-cli -- contract create 9.99 "{\"participants\": [{\"amount\": 9.99, \"address\": \"bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw\"}], \"radix\": 2}" --file="plugin-example/pkg/sapio_wasm_plugin_example_bg.wasm"` to see some magic!

You can use `cargo run --bin sapio-cli -- help` to learn more about what a the CLI can do! and `cargo run --bin sapio-cli -- <subcommand> help` to learn about subcommands like `contract`.

As a second experiment, try modifying the contract in plugin-example to one
of the contracts from sapio-contrib! Remember to recompile plugin-example
with `wasm-pack build`!

Still hungry for more? Implement your own smart contract idea -- you can use
sapio-contrib for inspiration or as building blocks for something new!

Stuck? Run `cargo doc --open --no-deps` to build and open the documentation
locally, or just shoot me a note and I'll guide you through it! Any and all
feedback welcome!
