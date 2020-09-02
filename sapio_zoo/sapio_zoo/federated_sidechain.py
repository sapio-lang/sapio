from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import List, Optional, Tuple
from sapio_compiler import (
    contract,
    Contract,
    Amount,
    Threshold,
    TransactionTemplate,
    Wait,
    Weeks,
)
from sapio_bitcoinlib.key import ECPubKey


@contract
class FederatedPegIn:
    keys: List[ECPubKey]
    thresh_all: int
    keys_backup: List[ECPubKey]
    thresh_backup: int
    amount: Amount


@FederatedPegIn.finish
def _(self):
    return Threshold(self.thresh_all, self.keys)

@FederatedPegIn.let
def backup_start(self):
    return Threshold(self.thresh_backup, self.keys_backup)

@backup_start
@FederatedPegIn.then
def backup(self):
    t = TransactionTemplate()
    t.add_output(
        self.amount,
        BackupOperator(BackupOperator.Props(
            keys=self.keys,
            thresh_all=self.thresh_all,
            keys_backup=self.keys_backup,
            thresh_backup=self.thresh_backup,
            amount=self.amount,
        )),
    )
    return t


@contract
class BackupOperator:
    keys: List[ECPubKey]
    thresh_all: int
    keys_backup: List[ECPubKey]
    thresh_backup: int
    amount: Amount

@BackupOperator.finish
def _(self):
    return Threshold(self.thresh_all, self.keys)

@BackupOperator.finish
def backup_finish(self):
    return Threshold(self.thresh_backup, self.keys_backup) & Wait(Weeks(4))
