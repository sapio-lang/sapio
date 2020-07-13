import unittest

from sapio_bitcoinlib.static_types import Sats
from sapio_zoo.p2pk import *
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestP2Pk(unittest.TestCase):
    def test(self):
        key = random_k()
        p = PayToPubKey(key=key, amount=Sats(10))
        p.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
