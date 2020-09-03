from sapio_compiler import *
from sapio_compiler import SignedBy, RevealPreImage


@contract
class SignedEscrow:
    alice: PubKey
    bob: PubKey
    escrow: Clause


"""
SignedEscrow uses an arbitrary Clause to unlock a uncooperative close with
either party. As such, several different types of Contract can be generated
with appropriate selection of escrow.

Examples
--------
Classic Signature

>>> SignedEscrow(alice=..., bob=..., escrow=SignedBy(escrow_key))

Either party after timeout.

>>> SignedEscrow(alice=..., bob=..., escrow=AfterClause(Weeks(1)))

Escrow multisig

>>> SignedEscrow(alice=..., bob=..., escrow=SignedBy(escrow_key_a) & SignedBy(escrow_key_b))

Binary Oracle Escrow

>>> SignedEscrow.BinaryEscrow(alice=..., bob=..., H1, H2)

"""


@SignedEscrow.finish
def uncooperative_close(self):
    """
    Pathway to be taken if cooperation cannot be reached between alice and bob
    """
    return (SignedBy(self.alice) | SignedBy(self.bob)) & SignedBy(self.escrow)


@SignedEscrow.finish
def cooperatative_close(self):
    """
    Pathway to be taken with cooperation between alice and bob
    """
    return SignedBy(self.alice) & SignedBy(self.bob)


def BinaryEscrow(alice: PubKey, bob: PubKey, alice_h: Hash, bob_h: Hash):
    """
    Convenience constructor for binary preimage oracles
    """
    return SignedEscrow(
        SignedProps(
            alice=alice,
            bob=bob,
            escrow=(SignedBy(alice) & RevealPreImage(alice_h))
            | (SignedBy(bob) & RevealPreImage(bob_h)),
        )
    )
