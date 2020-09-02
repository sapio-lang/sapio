import functools
from typing import List, Tuple

from bitcoin_script_compiler import (
    Wait,
    SignedBy,
    Weeks,
)
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_bitcoinlib.key import ECKey
from sapio_compiler import Contract, TransactionTemplate, contract


def segment_by_radix(L, n):
    size = max(len(L) // n, n)
    for i in range(0, len(L), size):
        if i + size + size > len(L):
            yield L[i:]
            return
        else:
            yield L[i: i + size]


@contract
class TreePay(Contract):
    payments: List[Tuple[Amount, Contract]]
    radix: int

@TreePay.then
def expand(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    segments = list(segment_by_radix(self.payments, self.radix))
    if len(segments) == 1:
        for payment in self.payments:
            tx.add_output(payment[0], payment[1])
    else:
        for segment in segments:
            amount = functools.reduce(
                lambda x, y: x + y, [a for (a, _) in segment], Amount(0)
            )
            tx.add_output(amount, TreePay(TreePay.Props(payments=segment, radix=self.radix)))
    return tx


# Mock!
def libsecp_make_musig():
    e = ECKey()
    e.generate()
    return e.get_pubkey()


@contract
class CollapsibleTree(Contract):
    payments: List[Tuple[Amount, Contract]]
    radix: int
    def get_musig(self) -> ECKey:
        return libsecp_make_musig()

@CollapsibleTree.then
def expand(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    segments = list(segment_by_radix(self.payments, self.radix))
    if len(segments) == 1:
        for payment in self.payments:
            tx.add_output(payment[0], payment[1])
    else:
        for segment in segments:
            amount = functools.reduce(
                lambda x, y: x + y, [a for (a, _) in segment], Amount(0)
            )
            tx.add_output(amount, CollapsibleTree(CollapsibleTree.Props(payments=segment, radix=self.radix)))
    return tx


@CollapsibleTree.finish
def cooperate_out(self):
    return SignedBy(self.get_musig())
