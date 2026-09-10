`ctvhash.json` is the complete, unmodified official BIP-119 hash corpus,
retrieved on 2026-09-07 from Bitcoin BIPs revision
`ae747e2b909ab5dd32632ed3a8b09839193d53e3`:

https://github.com/bitcoin/bips/blob/ae747e2b909ab5dd32632ed3a8b09839193d53e3/bip-0119/vectors/ctvhash.json

File SHA-256:
`3cff1abe3284b9d05fa95422724680f0ffdaa69675fc3c679de688c872174160`

The 100 transactions supply 400 expected hashes, including indices outside
the transaction's inputs. All four combinations of witness presence and
nonempty scriptSig presence have 25 transactions. Expected values come
from the reference implementation, not the Rust implementation under test.

BIP-119 is authored by Jeremy Rubin and licensed under BSD-3-Clause.

`bip446-basics.json` is the complete, unmodified 19-case BIP446 corpus,
retrieved on 2026-09-10:

https://github.com/bitcoin/bips/blob/master/bip-0446/basics.json

File SHA-256:
`19cd88462784a67e6614f19b991af81c66a671dec989ecdeebc21813b0ae4372`

Each selected input's script pushes an expected template hash before
`OP_TEMPLATEHASH OP_EQUAL`. The corpus covers transaction-field mutations,
input selection, annex inclusion and mutation, and fields intentionally not
committed by TEMPLATEHASH. Both the native message builder and the actual
WASM guest are checked against these cases. BIP446 is licensed under CC0-1.0.
