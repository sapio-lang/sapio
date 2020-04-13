from __future__ import annotations

from src.examples.basic_vault import Vault
from src.examples.p2pk import PayToPubKey
from src.examples.smarter_vault import SmarterVault
from src.examples.tree_pay import TreePay, CollapsibleTree
from src.examples.undo_send import UndoSend
# import sys; sys.exit()
from src.lib.bitcoinlib.messages import COutPoint
from src.lib.contract import TransactionTemplate
from src.lib.lang import Weeks


def main() -> None:
    key1 = b"0" * 32
    key2 = b"1" * 32
    key3 = b"2" * 32
    pk2 = PayToPubKey(key=key2, amount=10)
    u = UndoSend(to_key=key1, from_contract=pk2, amount=10, timeout=Weeks(6))
    pk1 = PayToPubKey(key=key1, amount=1)
    t= TransactionTemplate()
    v = Vault(cold_storage=pk1, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=1)
    t.add_output(10, v)
    print(v.bind(COutPoint(0, 0)))


def main2():
    key2 = b"1" * 32
    pk2 = PayToPubKey(key=key2, amount=10)
    import os
    payments = [(10, PayToPubKey(key=os.urandom(4), amount=10)) for _ in range(102)]
    CollapsibleTree(payments=payments, radix=4)
    TreePay(payments=payments, radix=4)

    def cold_storage(v):
        #TODO: Use a real PubKey Generator
        payments = [(v / 10, PayToPubKey(key=os.urandom(4), amount=v / 10)) for _ in range(10)]
        return TreePay(payments=payments, radix=4)
    SmarterVault(cold_storage=cold_storage, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=100)

    def cold_storage(v):
        #TODO: Use a real PubKey Generator
        return SmarterVault(cold_storage=lambda x: pk2, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=v / 10)
    s = SmarterVault(cold_storage=cold_storage, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=100)
    return s




if __name__ == "__main__":
    main()
    main2()
