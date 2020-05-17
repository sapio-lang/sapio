from sapio_stdlib.escrow import *
from sapio_stdlib.p2pk import P2PK
import unittest
from sapio_bitcoinlib.test_framework import BitcoinTestFramework
from sapio_bitcoinlib.util import assert_equal, wait_until


class TestEscrow(unittest.TestCase):
    def test_multisig(self):
        alice = P2PK(key=PubKey(b"a"))
        bob = P2PK(key=PubKey(b"b"))

        t = TransactionTemplate()
        t.add_output(Bitcoin(1), alice)
        t.add_output(Bitcoin(2), bob)
        t.set_sequence(Weeks(2))
        escrow = TrustlessEscrow(
            parties=[SignedBy(alice.key), SignedBy(bob.key)], default_escrow=t,
        )
        assert escrow.txn_abi[escrow.uncooperative_close.__func__][0] is t
        assert_equal(
            escrow.conditions_abi[escrow.uncooperative_close.__func__], Satisfied(),
        )
        coop_close = escrow.conditions_abi[escrow.cooperative_close.__func__]
        assert_equal(coop_close.left.pk, alice.key)
        assert_equal(coop_close.right.pk, bob.key)


if __name__ == "__main__":
    unittest.main()
