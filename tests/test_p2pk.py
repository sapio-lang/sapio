import unittest

from sapio.bitcoinlib.static_types import Sats
from sapio.examples.p2pk import *


class TestP2Pk(unittest.TestCase):
    def test(self):
        key1 = b"0" * 32
        key2 = b"1" * 32
        key3 = b"2" * 32
        PayToPubKey(key=key2, amount=Sats(10))

if __name__ == '__main__':
    unittest.main()
