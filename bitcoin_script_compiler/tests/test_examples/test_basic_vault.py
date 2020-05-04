
import unittest

from sapio.examples.p2pk import *
from sapio.examples.undo_send import *
from sapio.examples.basic_vault import *
from sapio.script.clause import Weeks
from sapio.contract.txtemplate import TransactionTemplate
from bitcoinlib.static_types import Sats, Bitcoin


class TestBasicVault(unittest.TestCase):
    def test_basic_vault(self):
        key1 = b"0" * 32
        key2 = b"1" * 32
        key3 = b"2" * 32
        pk2 = PayToPubKey(key=key2, amount=Sats(10))
        u = UndoSend(to_key=key1, from_contract=pk2, amount=Sats(10), timeout=Weeks(6))
        pk1 = PayToPubKey(key=key1, amount=1)
        t= TransactionTemplate()
        v = Vault(cold_storage=pk1, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=Bitcoin(1))
        t.add_output(v.amount_range[1], v)

if __name__ == '__main__':
    unittest.main()
