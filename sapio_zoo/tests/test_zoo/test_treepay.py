import unittest

from sapio_bitcoinlib.static_types import Sats, Bitcoin, PubKey
from sapio_zoo.p2pk import *
from sapio_zoo.tree_pay import *
from sapio_zoo.collapsible_tree import *
from bitcoin_script_compiler.clause import Weeks
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestTreePay(unittest.TestCase):
    def test_tree_pay(self):
        payments = [
            (Bitcoin(10), PayToPubKey.create(key=random_k(), amount=Bitcoin(10)))
            for _ in range(102)
        ]
        for radix in [2, 4, 25, 1000]:
            c = CollapsibleTree.create(payments=payments, radix=radix)
            t = TreePay.create(payments=payments, radix=radix)
            c.bind(COutPoint(0, 0))
            t.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
