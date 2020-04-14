from __future__ import annotations

import os
from functools import lru_cache
from sapio.contract import TransactionTemplate

from sapio.examples.undo_send import UndoSend
from sapio.examples.p2pk import PayToPubKey
from sapio.examples.tree_pay import TreePay, CollapsibleTree
from sapio.examples.basic_vault import Vault
from sapio.examples.smarter_vault import SmarterVault


# import sys; sys.exit()
from sapio.bitcoinlib.static_types import Sats, Bitcoin
from sapio.spending_conditions.script_lang import Weeks, Amount


def main() -> None:
    key1 = b"0" * 32
    key2 = b"1" * 32
    key3 = b"2" * 32
    pk2 = PayToPubKey(key=key2, amount=Sats(10))
    u = UndoSend(to_key=key1, from_contract=pk2, amount=Sats(10), timeout=Weeks(6))
    pk1 = PayToPubKey(key=key1, amount=1)
    t= TransactionTemplate()
    v = Vault(cold_storage=pk1, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=Bitcoin(1))
    t.add_output(v.amount_range[1], v)


def main2():
    payments = [(Bitcoin(10), PayToPubKey(key=os.urandom(32), amount=Bitcoin(10))) for _ in range(102)]
    for radix in [2, 4, 25, 1000]:
        CollapsibleTree(payments=payments, radix=radix)
        TreePay(payments=payments, radix=radix)

    key2 = os.urandom(32)


    @lru_cache()
    def cold_storage(v : Amount):
        #TODO: Use a real PubKey Generator
        payments = [(v // 10, PayToPubKey(key=os.urandom(32), amount=v // 10)) for _ in range(10)]
        return TreePay(payments=payments, radix=4)
    SmarterVault(cold_storage=cold_storage, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=Bitcoin(100))

    @lru_cache()
    def cold_storage2(v: Amount):
        #TODO: Use a real PubKey Generator
        return SmarterVault(cold_storage=cold_storage, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step= (v // 10))
    s = SmarterVault(cold_storage=cold_storage2, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=100)
    return s




if __name__ == "__main__":
    main()
    main2()
