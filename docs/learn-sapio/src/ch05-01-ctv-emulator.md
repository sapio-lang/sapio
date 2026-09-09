# BIP-119 Emulation

Changes to Bitcoin take a long time. The star player in making Sapio work is
BIP-119, and that might take a while to get merged. To get around this, Sapio
provides some tools to enable similar functionality today by emulating
BIP-119 with signatures.


## Compilation inputs and runtime signers

Compilation takes a public `LoweringPlan` containing either `Native` or
`CtvEmulation { signers, threshold }`. The signer entries are extended public
keys. Only explicit `Emulatable(Ctv(hash))` predicates follow that plan;
ordinary native CTV clauses and raw scripts retain their meaning. Transaction
actions add an emulatable CTV predicate automatically.

Module arguments must include `context.lowering`. Nested modules receive the
same explicit inputs, and compilation does not connect to a signer or ask a
host to choose a policy. WASM compilation hosts expose no signing capability.

The CTV emulator implementations provide runtime signing services. Binding
checks that the configured runtime signer reproduces the policies selected by
the artifact's recorded public inputs. Keeping the seed requires trusting its
custodian; deleting it requires collecting every signature that will be needed
first. Public lowering inputs alone do not establish availability or key deletion.


This crate also defines logic for servers that want to offer emulator
services to binding and signing clients. This is convenient since the emulator
server must be kept secure, so an organization may want it to be more tightly
safeguarded.

The emulator definitions include wrapper types that compose individual
instances of an emulator into a federated multisig. This is useful
for circumstances where a contract is between e.g. 2 parties and both
have an emulator server. Then the contract can be "immutable" unless
both collude.

Configure your own runtime signers separately from the compilation inputs.
The CLI requires an explicit signer or native research mode; a missing or
disabled emulator configuration no longer selects native CTV implicitly.

### How it works

*See the source code for more detailed documentation.*

CheckTemplateVerify essentially functions as a self-signed transaction. I.e.,
imagine you could create a public key that could only ever sign a transaction
which matched a certain pattern?

To implement this functionality, we use BIP-32 HD keys with public derivation.

On initialization, a server picks a seed S and generates a root public key K
from it, and publishes K.

Users generate a transaction T and extract the CheckTemplateVerify hash H for
it. They then take H and convert it into a derivation path D of 8 u32's and 1
u8 for non-hardened derivation (see `hash_to_child_vec`).

This derivation path is then applied to K to generate a key C. This key is
added with a CheckSig(SIGHASH_ALL) to the script in place of a CTV clause.

Then, when a user desires to spend an output with such a key, they create the
entire transaction they want to occur and send it to the emulator server.

Without even checking to see that the key is used in the transaction, the
server generates the template hash H' (which should equal H) and then signs,
returning the signature to the client.

Before creating a contract, clients may wish to collect all possible
signatures required to prevent an availability fault.

This scheme has the benefit that:

1. contract specification can occur without any online processes
1. The server has no intelligent logic, all guarantees are structural.
1. Server is completely stateless.
1. Availability/malfeasance can be controlled for with multisig
1. 1:1 functionality mapping to CTV

The downside of this approach to emulation is that:

1. It is somewhat inefficient for scripts which have many branched possibilities.
1. No inherent mechanism to delete keys after use to protect against future exfiltration.


#### Why BIP-32

We use BIP-32 because it is a well studied primitive and derivation paths are
compatible with existing signing hardware. While it is true that a tweak of
32 bytes could be directly applied to the key more efficiently, easier
interoperability with existing tools seemed to be the best path.


## Customizing Emulator Trait

The emulator trait belongs to binding and signing. `get_signer_for` advertises
the runtime's policy for compatibility checks, and `sign` supplies signatures.
Compilation derives policies locally from `LoweringPlan` and never invokes
this trait. A custom implementation does not add support for arbitrary
encumbrance programs; those require their own defined evaluator and protocol.

As a user of the Sapio library, you can define your own custom emulator logic
but that's out of scope of this book.

## Future Work

[There is a plan](https://github.com/sapio-lang/sapio/issues/100) to make
emulation more efficient based on Merkelization, but it is not yet
implemented because it messes with the current way the compiler works.

The efficiency issues are also solvable, more or less, with taproot.
