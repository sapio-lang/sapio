import unittest

from sapio_bitcoinlib.static_types import Sats
from sapio_zoo.p2pk import *


class TestP2Pk(unittest.TestCase):
    def test(self):
        key = b"1" * 32
        PayToPubKey(key=key, amount=Sats(10))


if __name__ == "__main__":
    unittest.main()
