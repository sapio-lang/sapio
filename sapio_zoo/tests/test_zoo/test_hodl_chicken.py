from functools import reduce

from sapio_compiler import *
from sapio_zoo.hodl_chicken import *
from sapio_bitcoinlib.address import key_to_p2pkh, key_to_p2wpkh
from sapio_stdlib.p2pk import P2PK
import os
import unittest
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestHodlChicken(unittest.TestCase):

    def test_hodl_chicken(self):
        alice_key = random_k()
        bob_key = random_k()

        hodl_chicken = HodlChicken(
            alice_key=alice_key, 
            bob_key=bob_key, 
            alice_deposit=Sats(100_000_000), 
            bob_deposit=Sats(100_000_000),
            winner_gets=Sats(50_000_000),
            chicken_gets=Sats(150_000_000))

        hodl_chicken.bind(COutPoint(0, 0))

if __name__ == "__main__":
    unittest.main()
