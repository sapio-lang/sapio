from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import List, Optional, Tuple

from sapio_compiler import *
from sapio_zoo.p2pk import PayToSegwitAddress


# Demonstrates multisig without using any special multisig functionality
class RawMultiSig(Contract):
    class Fields:
        keys: List[PubKey]
        thresh: int

    @unlock
    def _(self):
        return Threshold(self.thresh, self.keys)


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
        return Threshold(self.thresh_all, self.keys)

    @require
    def lower_threshold(self):
        return Threshold(self.thresh_path, self.keys)

    @lower_threshold
    @guarantee
    def redeem(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount, self.path)
        return tx
