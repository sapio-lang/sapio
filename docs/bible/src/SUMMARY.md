# The Sapio Language

[The Sapio Language](title-page.md)
[Introduction](./ch00-00-introduction.md)

# ABC 123
- [Getting Started](./ch01-00-getting-started.md)
    - [Installing Sapio](./ch01-01-installation.md)
    - [Learning Rust](./ch01-02-learn-rust.md)
    - [Hello World](./ch01-03-hello-world.md)


- [BIP-119 CTV Fundamentals](./ch02-00-bip-119.md)

- [Sapio Basics](./ch03-00-basics.md)
    - [Contract Guts](./ch03-01-guts.md)
        - [Miniscript/Policy](./ch03-01-miniscript.md)
        - [Template Builder](./ch03-01-builder.md)
        - [Time Locks](./ch03-01-timelocks.md)
        - [Sats and Coins](./ch03-01-amounts.md)
    - [Contract Actions](./ch03-02-guts.md)
        - [guard!](./ch03-02-guard.md)
        - [compile_if!](./ch03-02-compile_if.md)
        - [then!](./ch03-02-then.md)
        - [finish!](./ch03-02-finish.md)
        - [When to use macros?](./ch03-02-when-use-macros.md)
    - [Contract Declarations](./ch03-03-declarations.md)
    - [Contract Compilation Overview](./ch03-04-compliation.md)

# Exercises
- [Sapio for Fun (and Profit)](./ch04-00-sapio-fun-profit.md)

# Warnings
- [Limitations of Sapio](./ch05-00-limitations.md)
    - [BIP-119 Emulation](./ch05-01-ctv-emulator.md)
    - [No Taproot](./ch05-02-taproot.md)
    - [Advanced Transaction Handling](./ch05-03-txns.md)
    - [Mempool & Fees](./ch05-04-gas.md)

# Advanced Topics
- [Application Packaging](./ch06-00-packaging.md)
    - [WASM](./ch06-01-wasm.md)
    - [Rust Lib/Bin](./ch06-02-rust.md)
    - [TUX](./ch06-02-tux.md)

- [Sapio CLI](./ch07-00-cli.md)

- [Advanced Rust Patterns](./ch08-00-useful-rust.md)
    - [Type Level State Machines](./ch08-01-state-machines.md)
    - [TryFrom Constructors](./ch08-02-tryfrom.md)
    - [Concrete & Generic Types](./ch08-03-concrete.md)