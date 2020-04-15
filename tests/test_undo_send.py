import unittest

from sapio.bitcoinlib.static_types import Sats
from sapio.examples.p2pk import *
from sapio.examples.undo_send import *
from sapio.spending_conditions.script_lang import Weeks


class TestUndoSend(unittest.TestCase):
    def test_undo_send(self):
        key1 = b"0" * 32
        key2 = b"1" * 32
        key3 = b"2" * 32
        pk2 = PayToPubKey(key=key2, amount=Sats(10))
        u = UndoSend(to_key=key1, from_contract=pk2, amount=Sats(10), timeout=Weeks(6))

if __name__ == '__main__':
    unittest.main()
