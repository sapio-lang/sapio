import functools
from typing import List, Tuple

from bitcoin_script_compiler import (
    AfterClause,
    AssignedVariable,
    SignatureCheckClause,
    Weeks,
)
from bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, TransactionTemplate, guarantee, unlock


def segment_by_radix(L, n):
    size = max(len(L) // n, n)
    for i in range(0, len(L), size):
        if i + size + size > len(L):
            yield L[i:]
            return
        else:
            yield L[i : i + size]


class TreePay(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int

    @guarantee
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        segments = list(
            segment_by_radix(self.payments.assigned_value, self.radix.assigned_value)
        )
        if len(segments) == 1:
            for payment in self.payments.assigned_value:
                tx.add_output(payment[0], payment[1])
        else:
            for segment in segments:
                amount = functools.reduce(
                    lambda x, y: x + y, [a for (a, _) in segment], Amount(0)
                )
                tx.add_output(
                    amount, TreePay(payments=segment, radix=self.radix.assigned_value)
                )
        return tx


def libsecp_make_musig(x):
    return "0" * 32


class CollapsibleTree(Contract):
    class Fields:
        payments: List[Tuple[Amount, Contract]]
        radix: int

    @guarantee
    def expand(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        segments = list(
            segment_by_radix(self.payments.assigned_value, self.radix.assigned_value)
        )
        if len(segments) == 1:
            for payment in self.payments.assigned_value:
                tx.add_output(payment[0], payment[1])
        else:
            for segment in segments:
                amount = functools.reduce(
                    lambda x, y: x + y, [a for (a, _) in segment], Amount(0)
                )
                tx.add_output(
                    amount, TreePay(payments=segment, radix=self.radix.assigned_value)
                )
        return tx

    def get_musig(self) -> AssignedVariable[PubKey]:
        return AssignedVariable(PubKey(b"0" * 32), "musig")

    @unlock
    def cooperate_out(self):
        return SignatureCheckClause(self.get_musig())
