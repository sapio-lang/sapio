from __future__ import annotations

from typing import Callable

from lang import *
from util import *

from my_types import *


class Input:
    def __init__(self) -> None:
        self.witness: Stack = Stack([])
        self.script_signature: Script = Script(bytes())
        self.sequence: Sequence = Sequence(u32(c_uint32(0)))


class Output:
    def __init__(self):
        self.amount: Amount = Amount(i64(c_int64(0)))
        self.script: Script = Script(bytes())


class Transaction:
    def __init__(self) -> None:
        self.inputs: List[Input] = []
        self.outputs: List[Output] = []
        self.version: Version = Version(u32(c_uint32(2)))
        self.lock_time: LockTime = LockTime(u32(c_uint32(0)))

    def get_ctv_hash(self):
        import os
        return os.urandom(32)


from contract import *

class UndoSend(Contract):
    class Fields:
        from_contract: Contract
        to_key: PubKey
        amount: Amount
        timeout: TimeSpec

    @unlock(lambda self: AfterClause(self.timeout)*SignatureCheckClause(self.to_key))
    def _(self): pass

    @path(lambda self: SignatureCheckClause(self.to_key))
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount.value, self.from_contract.value)
        return tx


class Vault(Contract):
    class Fields:
        cold_storage: Contract
        hot_storage: Contract
        n_steps: int
        amount_step: Amount
        timeout: TimeSpec
        mature: TimeSpec

    @path
    def step(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount_step.value,
                      UndoSend(from_contract=self.cold_storage,
                               to_key=self.hot_storage,
                               timeout=self.mature,
                               amount=self.amount_step))
        if self.n_steps.value > 1:
            steps_left = self.n_steps.value - 1
            sub_amount = (self.n_steps.value-1) * self.amount_step.value
            sub_vault = Vault(cold_storage=self.cold_storage,
                                hot_storage=self.hot_storage,
                                n_steps=self.n_steps.value - 1,
                                timeout=self.timeout,
                                mature=self.mature,
                                amount_step=self.amount_step)
            tx.add_output(sub_amount, sub_vault)
        return tx

    @path
    def to_cold(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.n_steps.value * self.amount_step.value,
        self.cold_storage.value)
        return tx


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass

def segment_by_radix(L, n):
    size = max(len(L) // n, n)
    for i in range(0, len(L), size):
        yield L[i:i+size]
from typing import Tuple
class TreePay(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int
    @path
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        for segment in segment_by_radix(self.payments.value, self.radix.value):
            if len(segment) > self.radix.value:
                tx.add_output( sum(a for (a, _) in segment),
                    TreePay(payments=segment, radix=self.radix.value)
                )
            else:
                for payment in segment:
                    tx.add_output(payment[0], payment[1])
        return tx

def libsecp_make_musig(x):
    return "0"*32

class CollapsibleTree(Contract):
    class Fields:
        payments: List[Tuple[Amount, PubKey]]
        radix: int
    @path(lambda _: AfterClause(Weeks(2)))
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        for segment in segment_by_radix(self.payments.value, self.radix.value):
            if len(segment) > self.radix.value:
                tx.add_output( sum(a for (a, _) in segment),
                               CollapsibleTree(payments=segment, radix=self.radix.value)
                               )
            else:
                for payment in segment:
                    tx.add_output(payment[0], payment[1])
        return tx
    def get_musig(self) -> Variable[PubKey]:
        return Variable("musig", b"0"*32)

    @unlock(lambda self: SignatureCheckClause(self.get_musig()))
    def _(self):pass


#import sys; sys.exit()




def main() -> None:
    key1 = b"0" * 32
    key2 = b"1" * 32
    key3 = b"2" * 32
    pk2 = PayToPubKey(key=key2, amount=10)
    u = UndoSend(to_key=key1, from_contract=pk2, amount=10, timeout=Weeks(6))
    pk1 = PayToPubKey(key=key1, amount=1)
    t= TransactionTemplate()
    t.add_output(10, Vault(cold_storage=pk1, hot_storage=key2, n_steps=10, timeout=Weeks(1), mature=Weeks(2), amount_step=1))

    import os
    payments = [(10, PayToPubKey(key=os.urandom(4), amount=10)) for _ in range(102)]
    CollapsibleTree(payments=payments, radix=4)
    TreePay(payments=payments, radix=4)


if __name__ == "__main__":
    main()
