# Sapio CTV Emulators


Sapio CTV Emulators defines implementations of the emualator trait that can
be used by sapio compiler library users. This includes wrapper types that
compose instances of an emulator into a federated multisig.

This crate also defines logic for servers that want to offer emulator services.

See [Sapio CLI](../cli/README.md) for how to run a server.


## How it works

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
entire transaction they want to occur and send it to the the emualtor server.

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


### Why BIP-32

We use BIP-32 because it is a well studied primitive and derivation paths are
compatible with existing signing hardware. While it is true that a tweak of
32 bytes could be directly applied to the key more efficiently, easier
interoperability with existing tools seemed to be the best path.