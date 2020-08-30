from sapio_stdlib.escrow import *
from sapio_stdlib.p2pk import P2PK
import unittest
from sapio_bitcoinlib.test_framework import BitcoinTestFramework
from sapio_bitcoinlib.util import assert_equal, wait_until
from .util import random_k


class TestEscrow(unittest.TestCase):
    def test_multisig(self):
        alice = P2PK.create(key=random_k(), amount=Bitcoin(1))
        bob = P2PK.create(key=random_k(), amount=Bitcoin(2))

        t = TransactionTemplate()
        t.add_output(Bitcoin(1), alice)
        t.add_output(Bitcoin(2), bob)
        t.set_sequence(Weeks(2))
        escrow = TrustlessEscrow.create(
            parties=[SignedBy(alice.data.key), SignedBy(bob.data.key)], default_escrow=t
        )
        assert escrow.txn_abi["uncooperative_close"][1][0] is t
        assert_equal(
            escrow.conditions_abi["uncooperative_close"][1], Satisfied(),
        )
        coop_close = escrow.conditions_abi["cooperative_close"][1]
        assert_equal(coop_close.left.pubkey, alice.data.key)
        assert_equal(coop_close.right.pubkey, bob.data.key)


if __name__ == "__main__":
    unittest.main()
