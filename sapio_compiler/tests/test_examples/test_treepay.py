

import unittest

from bitcoinlib.static_types import Sats, Bitcoin
from sapio_compiler.examples.p2pk import *
from sapio_compiler.examples.tree_pay import *
from bitcoin_script_compiler.clause import Weeks

import os

class TestTreePay(unittest.TestCase):
    def test_tree_pay(self):
        payments = [(Bitcoin(10), PayToPubKey(key=os.urandom(32), amount=Bitcoin(10))) for _ in range(102)]
        for radix in [2, 4, 25, 1000]:
            CollapsibleTree(payments=payments, radix=radix)
            TreePay(payments=payments, radix=radix)

if __name__ == '__main__':
    unittest.main()
