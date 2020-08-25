from functools import reduce

from sapio_compiler import *
from sapio_zoo.hodl_chicken import *
from sapio_bitcoinlib.address import key_to_p2pkh, key_to_p2wpkh, script_to_p2wsh
from sapio_bitcoinlib.script import CScript
from sapio_zoo.p2pk import PayToSegwitAddress
import os
import unittest
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint

class TestHodlChicken(unittest.TestCase):

    def test_hodl_chicken(self):
        alice_script = script_to_p2wsh(CScript([b"Alice's Key Goes Here!"]))
        bob_script = script_to_p2wsh(CScript([b"Bob's Key Goes Here!"]))
        alice_key = random_k()
        bob_key = random_k()

        hodl_chicken = HodlChicken(
            alice_contract=lambda x: PayToSegwitAddress(amount=AmountRange.of(x), address=alice_script), 
            bob_contract=lambda x: PayToSegwitAddress(amount=AmountRange.of(x), address=bob_script), 
            alice_key=alice_key,
            bob_key=bob_key,
            alice_deposit=Sats(100_000_000),
            bob_deposit=Sats(100_000_000),
            winner_gets=Sats(50_000_000),
            chicken_gets=Sats(150_000_000))

        hodl_chicken.bind(COutPoint(0, 0))

if __name__ == "__main__":
    unittest.main()
