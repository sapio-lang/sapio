from functools import reduce

from sapio_compiler import *
from sapio_zoo.federated_sidechain import *
import os
import unittest
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestMultiSig(unittest.TestCase):
    def test_multisig(self):
        a = [random_k() for _ in range(4)]
        b = [random_k() for _ in range(3)]
        f = FederatedPegIn(
            keys=a, thresh_all=3, keys_backup=b, thresh_backup=2, amount=Bitcoin(1)
        )
        f.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
