from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import List, Optional, Tuple

from sapio_compiler import *
from sapio_zoo.p2pk import PayToSegwitAddress


# Demonstrates multisig without using any special multisig functionality
@contract
class RawMultiSig:
    keys: List[PubKey]
    thresh: int


@RawMultiSig.finish
def _(self):
    return Threshold(self.thresh, self.keys)


# Demonstrates multisig with a default path accessible at a lower threshold
@contract
class RawMultiSigWithPath:
    keys: List[PubKey]
    thresh_all: int
    thresh_path: int
    path: Contract
    amount: Amount

@RawMultiSigWithPath.finish
def _(self):
    return Threshold(self.thresh_all, self.keys)

@RawMultiSigWithPath.let
def lower_threshold(self):
    return Threshold(self.thresh_path, self.keys)

@lower_threshold
@RawMultiSigWithPath.then
def redeem(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.amount, self.path)
    return tx
