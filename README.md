![](https://github.com/sapio-lang/sapio/workflows/Continuous%20integration/badge.svg)
# Sapio Workspace

Welcome!

Sapio is a framework for creating composable multi-transaction Bitcoin Smart Contracts.

This crate is a workspace for various Sapio Components such as:

1. [Sapio Language](sapio/README.md): Base Specification for Sapio Language and Contract Generation
1. [Sapio Contrib](sapio-contrib/README.md): Contract modules / functionality made available for general use
1. [Sapio Front](sapio-front/README.md): Protocols for interacting with a compilation session
1. [Sapio Compiler Server](sapio-ws/README.md): Binary for a websocket server running sapio-front

### Why is Sapio Different?
Sapio helps you build payment protocol specifiers that oblivious third parties
can participate in being none the wiser.

For example, with Sapio you can generate an address that represents a lightning
channel between you and friend and give that address to a third party service
like an exchange and have them create the channel without requiring any
signature interaction from you or your friend, zero trusted parties, and an
inability to differentiate your address from any other.

That's the tip of the iceberg of what Sapio lets you accomplish. See the [Sapio
Readme](sapio/README.md) to learn more.


