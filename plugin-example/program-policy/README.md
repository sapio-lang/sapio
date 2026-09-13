# Emulated program policy

`DelayedProgram` combines a native CSV delay with a supplied emulated program.
Compilation retains the full program, evaluator identity,
parameters and oracle root alongside its exact script-path signature slot.

The catalog fixture uses a short opaque program and evaluator identity to check
real WASM compilation, serialization and host validation. Spending requires a
matching registered evaluator and explicit authorization evidence, supplied
separately to the signer.
