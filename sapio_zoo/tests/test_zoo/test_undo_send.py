import unittest

from sapio_bitcoinlib.static_types import Sats
from sapio_zoo.p2pk import *
from sapio_zoo.undo_send import *
from bitcoin_script_compiler.clause import Weeks
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestUndoSend(unittest.TestCase):
    def test_undo_send(self):
        key1 = random_k()
        key2 = random_k()
        pk2 = PayToPubKey.create(key=key2, amount=Sats(10))
        u = UndoSend.create(
            to_key=key1, from_contract=pk2, amount=Sats(10), timeout=Weeks(6)
        )
        u2 = UndoSend2.create(
            to_contract=u, from_contract=pk2, amount=Sats(10), timeout=Weeks(6)
        )

        u2.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
