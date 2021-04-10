# Miniscript & Policy

Miniscript & Policy are tools for creating well formed Bitcoin scripts
developed by Blockstream developers Pieter Wiulle, Andrew Poelstra, and
Sanket Kanjalkar.

from the [miniscript website](http://bitcoin.sipa.be/miniscript/):

> Miniscript is a language for writing (a subset of) Bitcoin Scripts in a
> structured way, enabling analysis, composition, generic signing and more.
> 
> Bitcoin Script is an unusual stack-based language with many edge cases,
> designed for implementing spending conditions consisting of various
> combinations of signatures, hash locks, and time locks. Yet despite being
> limited in functionality it is still highly nontrivial to:
> 
> 1.    Given a combination of spending conditions, finding the most economical script to implement it.
> 1.    Given two scripts, construct a script that implements a composition of their spending conditions (e.g. a multisig where one of the "keys" is another multisig).
> 1.    Given a script, find out what spending conditions it permits.
> 1.    Given a script and access to a sufficient set of private keys, construct a general satisfying witness for it.
> 1.    Given a script, be able to predict the cost of spending an output.
> 1.    Given a script, know whether particular resource limitations like the ops limit might be hit when spending.
> 
> Miniscript functions as a representation for scripts that makes these sort of
> operations possible. It has a structure that allows composition. It is very
> easy to statically analyze for various properties (spending conditions,
> correctness, security properties, malleability, ...). It can be targeted by
> spending policy compilers (see below). Finally, compatible scripts can easily
> be converted to Miniscript form - avoiding the need for additional metadata
> for e.g. signing devices that support it.

For Sapio, we use a customized
[rust-miniscript](https://github.com/sapio-lang/rust-miniscript) which
extends miniscript with functionality relevent to CheckTemplateVerify and
Sapio. All changes should be able to be upstreamed eventually.

The Policy type (named Clause in Sapio) allows us to specify the predicates upon which various state transitions should unlock.

This makes it so that Sapio should be compatible with other software that can
generate valid Policies, and compatible with PSBT signing devices that
understand how to satisfy miniscripts.

A limitation of this approach is that there are certain types of script which
are possible, but not yet supported in Sapio. For example, the `OP_SIZE`
coin flip script is not currently possible with Miniscript.
