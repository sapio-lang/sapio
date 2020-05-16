from typing import List

from sapio_compiler import *


class SignedEscrow(Contract):
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

    class Fields:
        alice: PubKey
        bob: PubKey
        escrow: Clause

    @unlock
    def uncooperative_close(self):
        """
        Pathway to be taken if cooperation cannot be reached between alice and bob
        """
        return (SignedBy(self.alice) | SignedBy(self.bob)) & SignedBy(self.escrow)

    @unlock
    def cooperatative_close(self):
        """
        Pathway to be taken with cooperation between alice and bob
        """
        return SignedBy(self.alice) & SignedBy(self.bob)

    @staticmethod
    def BinaryEscrow(alice: PubKey, bob: PubKey, alice_h: Hash, bob_h: Hash):
        """
        Convenience constructor for binary preimage oracles
        """
        return SignedEscrow(
            alice=alice,
            bob=bob,
            escrow=(SignedBy(alice) & RevealPreImage(alice_h))
            | (SignedBy(bob) & RevealPreImage(bob_h)),
        )


class TrustlessEscrow(Contract):
    """
    An trustless escrow where the default resolution is a passed in is a transaction
    template to create

    Examples
    --------
    Close and pay Alice 1 btc, and Bob 2 btc.

    >>> t = TransactionTemplate()
    >>> t.add_output(Bitcoin(1), P2PK(key=alice))
    >>> t.add_output(Bitcoin(2), P2PK(key=bob))
    >>> TrustlessEscrow(parties=[alice, bob], default_escrow=t)

    Close and pay Alice 1 btc, and Bob 2 btc after 1 week.

    >>> t = TransactionTemplate()
    >>> t.add_output(Bitcoin(1), P2PK(key=alice))
    >>> t.add_output(Bitcoin(2), P2PK(key=bob))
    >>> t.set_sequence(Days(10))
    >>> TrustlessEscrow(parties=[alice, bob], default_escrow=t)

    Recursive Escrow, allows sub-parties to attempt cooperation.

    >>> t_ab = TransactionTemplate()
    >>> t_ab.add_output(Bitcoin(1), P2PK(key=alice))
    >>> t_ab.add_output(Bitcoin(2), P2PK(key=bob))
    >>> e_ab = TrustlessEscrow(parties=[alice, bob], default_escrow=t_ab)
    >>> t_cd = TransactionTemplate()
    >>> t_cd.add_output(Bitcoin(3), P2PK(key=carol))
    >>> t_cd.add_output(Bitcoin(4), P2PK(key=dave))
    >>> e_cd = TrustlessEscrow(parties=[carol, dave], default_escrow=t_cd)
    >>> t_abcd = TransactionTemplate()
    >>> t_abcd.add_output(Bitcoin(3), t_ab)
    >>> t_abcd.add_output(Bitcoin(7), t_cd)
    >>> TrustlessEscrow(parties=[alice, bob, carol, dave], default_escrow=t_abcd)
    """

    class Fields:
        parties: List[Clause]
        default_escrow: TransactionTemplate

    @guarantee
    def uncooperative_close(self) -> TransactionTemplate:
        return self.default_escrow

    @unlock
    def cooperative_close(self) -> Clause:
        ret = Satisfied()
        for cl in self.parties:
            ret &= cl
        return ret
