from typing import Tuple, List

from sapio.bitcoinlib.static_types import PubKey
from sapio import Contract, Amount, TransactionTemplate, path, unlock, Variable
from sapio.script_lang import AfterClause, Weeks, SignatureCheckClause
import functools


def segment_by_radix(L, n):
    size = max(len(L) // n, n)
    for i in range(0, len(L), size):
        yield L[i:i+size]


class TreePay(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int
    @path
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        for segment in segment_by_radix(self.payments.assigned_value, self.radix.assigned_value):
            if len(segment) > self.radix.assigned_value:
                tx.add_output(
                    functools.reduce(lambda x,y: x+y, [a for (a, _) in segment], Amount(0)),
                    TreePay(payments=segment, radix=self.radix.assigned_value)
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
        for segment in segment_by_radix(self.payments.assigned_value, self.radix.assigned_value):
            if len(segment) > self.radix.assigned_value:
                tx.add_output(
                               functools.reduce(lambda x, y: x + y, [a for (a, _) in segment], Amount(0)),
                               CollapsibleTree(payments=segment, radix=self.radix.assigned_value)
                               )
            else:
                for payment in segment:
                    tx.add_output(payment[0], payment[1])
        return tx
    def get_musig(self) -> Variable[PubKey]:
        return Variable("musig", b"0"*32)

    @unlock(lambda self: SignatureCheckClause(self.get_musig()))
    def _(self):pass