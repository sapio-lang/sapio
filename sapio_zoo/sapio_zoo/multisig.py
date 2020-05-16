from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import List, Optional, Tuple

from sapio_compiler import *
from sapio_zoo.p2pk import PayToSegwitAddress


def multisig(l, n):
    assert len(l) > n
    assert n > 0
    l2 = [SignedBy(v) for i, v in enumerate(l)]
    l3 = [
        reduce(lambda a, b: a & b, combo[1:], combo[0]) for combo in combinations(l2, n)
    ]
    return reduce(lambda a, b: a | b, l3[1:], l3[0])


# Demonstrates multisig without using any special multisig functionality
class RawMultiSig(Contract):
    class Fields:
        keys: List[PubKey]
        thresh: int

    @unlock
    def _(self):
        return multisig(self.keys, self.thresh)


# Demonstrates multisig with a default path accessible at a lower threshold
class RawMultiSigWithPath(Contract):
    class Fields:
        keys: List[PubKey]
        thresh_all: int
        thresh_path: int
        path: Contract
        amount: Amount

    @unlock
    def _(self):
        return multisig(self.keys, self.thresh_all)

    @require
    def lower_threshold(self):
        return multisig(self.keys, self.thresh_path)

    @lower_threshold
    @guarantee
    def redeem(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount, self.path)
        return tx
