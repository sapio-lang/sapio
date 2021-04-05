# Contract Compilation Overview

When the compiler sees a new contract, it proceeds by processing each path
item one at a time. If the order of compilation is important for your contract:

1. reconsider your priorities
1. repeat step 1
1. read the logic inside of the `Compilable::compile` function

This logic may be improved over time to take advantage of parallelization or
otherwise restructure. As such, one should be careful when switching compiler
versions.


## Determinism?

Sapio is designed to be determinism-friendly. Repeated runs of the same
program should -- unless the user includes entropy -- return the same
results.

However, at writing, this property is not closely audited for, so outputs
should be treated as required to be stored in order to use a contract.

On the other hand, determinism means that for multi-party contracts being
generated in a Replicated state machine, if all parties have the same e.g.
WASM plugin, they can generate a contract definition and check that the
merkle root (in this case, a bitcoin address) is the same. If it differs,
either the arguments differed, someone cheated, or there was unexpected
non-determinism.