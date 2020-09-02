from typing import List

from sapio_compiler import *
from sapio_compiler import SignedBy, RevealPreImage


@contract
class TrustlessEscrow:
    parties: List[Clause]
    default_escrow: TransactionTemplate


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


@TrustlessEscrow.then
def uncooperative_close(self) -> TransactionTemplate:
    return self.default_escrow


@TrustlessEscrow.finish
def cooperative_close(self) -> Clause:
    ret = Satisfied()
    for cl in self.parties:
        ret &= cl
    return ret
