# Sapio WASM Plugin Library

The Sapio WASM Plugin Library is a crate that can be depended on by either
clients or hosts for running Sapio WASM. It contains both the host
environment bindings and the ability to load a plugin using the Wasmer WASM
runtime.

# Stability
This is currently completely unstable. This means that artifacts built with
WASM should only be expected to be able to run on a corresponding commit hash
for the host software. Expect breaking changes!