from functools import reduce

from sapio_zoo.subcondition_check import SubCondition, ChecksSubCondition
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
        alice_timeout = 10
        bob_timeout = 6

        ChecksSubCondition.create(a_f = lambda a: SubCondition.create(k=alice_key, timeout_=alice_timeout),
                                  b_f = lambda a: SubCondition.create(k=bob_key, timeout_=bob_timeout),
                                  c =  10)

        try:
            ChecksSubCondition.create(a_f = lambda a: SubCondition.create(k=alice_key, timeout_=alice_timeout),
                                    b_f = lambda a: SubCondition.create(k=bob_key, timeout_=alice_timeout),
                                    c =  10)
        except AssertionError as e:
            assert 'check_timing' in str(e)



if __name__ == "__main__":
    unittest.main()
