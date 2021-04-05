# Advanced Transaction Handling

Sapio does not try to handle all possible types of Bitcoin transaction.

There are certain "advanced techniques" that have use cases, but are
difficult to reason about. For example, there are many ways that SIGHASH
flags can be exploited to create all sorts of possibilities. You can use
`OP_2DUP OP_SHA256 <H1> OP_EQUALVERIFY OP_SWAP OP_SHA256 <H2> OP_EQUALVERIFY OP_SIZE OP_SWAP OP_SIZE OP_EQUAL` (or something similar) to flip a fair coin between participants. There is a *lot*.

But Sapio doesn't make an effort to cleanly handle all possible contracts. It
makes an effort to address a safe and useful subset and make those contracts
well integrated with other standard software.

If you identify a killer use-case contract, please open an issue or a PR to
discuss the new functionality and how to add it.