# Build and spend your first contract

The starter's payment contract fixes a recipient, minimum amount, evaluator and
public oracle root. A typed action proposes payments under that fixed policy.
Its contract definition is separate from synthetic funding, private demonstration
keys, file handling and tests.

This source is included directly from the generated project's contract module:

```rust,ignore
{{#include ../../../cli/templates/starter/src/contract.rs}}
```

`#[sapio::contract]` preserves ordinary Rust methods. The `#[policy]` method
defines authorization; `#[action(suggested)]` exposes a typed `PaymentRequest`.
`TemplatePlan` declares exact recipient funding, change and a local fee
reservation. A proposal below the fixed minimum can be constructed, but the
evaluator rejects its signing request. Changing a proposed amount does not
change the contract's address.

Continue from the project created in the installation chapter with its
generated README, also available as the
[starter walkthrough](https://github.com/sapio-lang/sapio/blob/master/cli/templates/starter/README.md).
It creates the demo directory and gives the exact `contract explain`,
`contract spend` and explicit local
`signer program` commands. You can stop between commands and resume with the
immutable artifact/intent and latest partial PSBT.

Bitcoin validates an ordinary Taproot signature here. The oracle enforces the
WASM payment predicate; the chain does not execute it. Funding and keys are
synthetic demonstration inputs, and nothing is broadcast.

After completing the walkthrough, change `PaymentRequest` and its `pay` method,
then update `src/tests.rs` to express the behavior you intend. The surrounding
book retains historical examples and conceptual sketches; use this maintained
starter as the reference for a complete, runnable first project.
