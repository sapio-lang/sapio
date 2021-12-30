# Installing Sapio

## Sapio Pod QuickStart:

[**DOWNLOAD THE POD**](https://hub.docker.com/repository/docker/sapiolang/sapio)

Today, Sapio can come to you in an easy to set up Docker compatible container
(unofficial™). With the Sapio pod you get:

1. A CTV Compatible Bitcoin Node running regtest
2. Rust
3. A pre-build cached Sapio Directory for you to use as a workspace
4. sapio-cli pre-built
5. Sapio Studio built and running over X11 connected to your regtest node
6. neovim for editing

See [the repo](https://github.com/jeremyrubin/sapio-pod) for setup instructions,
especially with x11 through containerization.

This  is the simplest way to get a working Sapio playground, but you may prefer
to have it set up locally (x11 can be glitchy). The Sapio Pod is currently
targetted at someone wanting a pain free development environment for tutorials,
but future releases may target more specific needs such as deployments in
infrastructure.

The book will *assume* this is your setup, and instructions will be tailored
appropriately.

## Local QuickStart:

Sapio should work on all platforms, but is recommended for use with Linux (Ubuntu preferred).
Follow this quickstart guide to get going.

1.  Get [rust](https://rustup.rs/) if you don't have it already.
1.  Add the wasm target by running the below command in your terminal:
```bash
rustup target add wasm32-unknown-unknown
```
1. Tip: On an M1 Mac you may need to do the following:
> ```bash
> brew install llvm
> cargo install wasm-pack
> rustup toolchain install nightly
> ```
> and then load the following before compiling to use the newer llvm/clang.
> ```bash
> export PATH="/opt/homebrew/opt/llvm/bin:$PATH"
> export CC=/opt/homebrew/opt/llvm/bin/clang
> export AR=/opt/homebrew/opt/llvm/bin/llvm-ar
> rustup default nightly
> ```

1.  Clone this repo: 
```
git clone --depth 1 git@github.com:sapio-lang/sapio.git && cd sapio
```
We recommend a shallow clone unless you want the full history.
1.  Build a plugin
```
cd plugin-example/ && cargo build --target wasm32-unknown-unknown && cd ..
```
1.  Instantiate a contract from the plugin:
```
cargo run --bin sapio-cli -- contract create "{\"arguments\":{\"ForAddress\":{\"amount_step\":{\"Sats\":100},\"cold_storage\":\"bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj\",\"hot_storage\":\"bcrt1qumrrqgt7e3a7damzm8x97m6sjs20u8hjw2hcjj\",\"mature\":{\"RH\":10},\"n_steps\":10,\"timeout\":{\"RH\":5}}},\"context\":{\"amount\":1,\"network\":\"Regtest\"}}" --file="plugin-example/target/wasm32-unknown-unknown/debug/sapio_wasm_vault.wasm"
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


The first time you run it you most likely *will* have some errors, you will need
to ensure you've configured your client correctly. You can do this by opening
the Preferences menu and configuring it appropriately. Soon there will be a better
interface for first run setup.


## Docs

You can review the docs either by building them locally or viewing
[online](https://docs.rs/sapio).
