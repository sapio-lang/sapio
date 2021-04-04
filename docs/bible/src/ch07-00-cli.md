# Sapio Command Line Interface (CLI)

The Sapio CLI (or `sapio-cli`) is rapidly changing, but it is self
documenting using `cargo run sapio-cli help`.

`sapio-cli` aids users in:

1. compiling sapio contracts into templates
1. binding compiled templates to specific utxos from your bitcoin wallet
1. inspecting contract plugins
1. running emulator servers

`sapio-cli` has a config file (location dependent on platform, under
`org.judica.sapio-cli` e.g. `/home/<usr>/.config/sapio-cli/config.json`). The
config file can be overriden with the `-c` flag. This file allows users to set parameters
for compilation around:

1. to use regtest/mainnet/signet/etc
1. bitcoind to connect to & auth
1. CTV emulator servers to use
1. key-value mapping of nicknames to [WASM](./ch06-01-wasm.md) plugin hashes.
