import unittest

from sapio_bitcoinlib.static_types import Sats
from sapio_zoo.p2pk import *
from sapio_zoo.undo_send import *
from bitcoin_script_compiler.clause import Weeks


class TestUndoSend(unittest.TestCase):
    def test_undo_send(self):
        key1 = b"0" * 32
        key2 = b"1" * 32
        pk2 = PayToPubKey(key=key2, amount=Sats(10))
        u = UndoSend(to_key=key1, from_contract=pk2, amount=Sats(10), timeout=Weeks(6))
        u2 = UndoSend2(
            to_contract=u, from_contract=pk2, amount=Sats(10), timeout=Weeks(6)
        )


if __name__ == "__main__":
    unittest.main()
