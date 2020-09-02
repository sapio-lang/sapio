import unittest

from sapio_bitcoinlib.static_types import Sats, Bitcoin
from sapio_zoo.p2pk import *
from sapio_zoo.tree_pay import *
from sapio_zoo.smarter_vault import *
from bitcoin_script_compiler.clause import Weeks
from sapio_bitcoinlib.messages import COutPoint

import os
from functools import lru_cache
from .testutil import random_k


class TestSmarterVault(unittest.TestCase):
    def test_smarter_vault(self):
        key2 = PayToPubKey.create(key=random_k(), amount=Bitcoin(100))

        @lru_cache()
        def cold_storage(v: Amount):
            payments = [
                (v // 10, PayToPubKey.create(key=random_k(), amount=v // 10))
                for _ in range(10)
            ]
            return TreePay.create(payments=payments, radix=4)

        SmarterVault.create(
            cold_storage=cold_storage,
            hot_storage=key2,
            n_steps=10,
            timeout=Weeks(1),
            mature=Weeks(2),
            amount_step=Bitcoin(100),
        )

        @lru_cache()
        def cold_storage2(v: Amount):
            return SmarterVault.create(
                cold_storage=cold_storage,
                hot_storage=key2,
                n_steps=10,
                timeout=Weeks(1),
                mature=Weeks(2),
                amount_step=(v // 10),
            )

        s = SmarterVault.create(
            cold_storage=cold_storage2,
            hot_storage=key2,
            n_steps=10,
            timeout=Weeks(1),
            mature=Weeks(2),
            amount_step=100,
        )

        s.bind(COutPoint(0, 0))


if __name__ == "__main__":
    unittest.main()
