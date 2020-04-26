import functools
from typing import Tuple, List

from sapio.bitcoinlib.static_types import PubKey, Amount
from sapio.contract import Contract, path, TransactionTemplate, unlock
from sapio.script.clause import AfterClause, Weeks, SignatureCheckClause
from sapio.script.variable import AssignedVariable


def segment_by_radix(L, n):
    size = max(len(L) // n, n)
    for i in range(0, len(L), size):
        if i+size+size > len(L):
            yield L[i:]
            return
        else:
            yield L[i:i+size]


class TreePay(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int
    @path
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        segments = list(segment_by_radix(self.payments.assigned_value, self.radix.assigned_value))
        if len(segments) == 1:
            for payment in self.payments.assigned_value:
                tx.add_output(payment[0], payment[1])
        else:
            for segment in segments:
                amount = functools.reduce(lambda x, y: x + y, [a for (a, _) in segment], Amount(0))
                tx.add_output(amount, TreePay(payments=segment, radix=self.radix.assigned_value))
        return tx


def libsecp_make_musig(x):
    return "0"*32


class CollapsibleTree(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int
    @path
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        segments = list(segment_by_radix(self.payments.assigned_value, self.radix.assigned_value))
        if len(segments) == 1:
            for payment in self.payments.assigned_value:
                tx.add_output(payment[0], payment[1])
        else:
            for segment in segments:
                amount = functools.reduce(lambda x, y: x + y, [a for (a, _) in segment], Amount(0))
                tx.add_output(amount, TreePay(payments=segment, radix=self.radix.assigned_value))
        return tx
    def get_musig(self) -> AssignedVariable[PubKey]:
        return AssignedVariable(PubKey(b"0" * 32), "musig")

    @unlock(lambda self: SignatureCheckClause(self.get_musig()))
    def _(self):pass