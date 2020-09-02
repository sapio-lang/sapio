from sapio_compiler import *
from sapio_zoo.multisig import *
import unittest
from .testutil import random_k
from sapio_bitcoinlib.messages import COutPoint


class TestMultiSig(unittest.TestCase):
    def test_multisig(self):
        a = RawMultiSig.create(keys=[random_k() for _ in range(5)], thresh=2)
        b = RawMultiSigWithPath.create(
            keys=[random_k() for _ in range(5)],
            thresh_all=3,
            thresh_path=2,
            amount=Bitcoin(5),
            path=a,
        )
        a.bind(COutPoint(0, 0))
        b.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
