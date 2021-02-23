# Sapio CTV Emulator Trait

This emulator trait crate is a base that exports a trait definition and some
helper structs that are needed across the sapio ecosystem.

Defining the trait in its own crate allows us to use trait objects in our
compiler internals without needing to have the compiler directly depend on
e.g. networking primitives.