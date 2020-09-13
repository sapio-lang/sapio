
from functools import reduce

from sapio_zoo.recomputation import Recomputation
from sapio_bitcoinlib.address import key_to_p2pkh, key_to_p2wpkh, script_to_p2wsh
from sapio_bitcoinlib.script import CScript
import os
import unittest
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestHodlChicken(unittest.TestCase):
    def test_subcondition_check(self):
        alice_key = random_k()
        bob_key = random_k()
        Recomputation.create(k1=alice_key, k2=bob_key, amount=20)


if __name__ == "__main__":
    unittest.main()
