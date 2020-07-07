
from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import List, Optional, Tuple
from sapio_compiler import Contract, Amount, guarantee, unlock, Threshold, require, TransactionTemplate, Wait, Weeks
from sapio_bitcoinlib.key import ECPubKey


class FederatedPegIn(Contract):
    class Fields:
        keys: List[ECPubKey]
        thresh_all: int
        keys_backup: List[ECPubKey]
        thresh_backup: int
        amount: Amount

    @unlock
    def _(self):
        return Threshold(self.thresh_all, self.keys)

    @require
    def backup_start(self):
        return Threshold(self.thresh_backup, self.keys_backup)

    @backup_start
    @guarantee
    def backup(self):
        t = TransactionTemplate()
        t.add_output(self.amount, BackupOperator(keys=self.keys,
                                                 thresh_all=self.thresh_all,
                                                 keys_backup=self.keys_backup,
                                                 thresh_backup=self.thresh_backup,
                                                 amount=self.amount))
        return t


class BackupOperator(Contract):
    class Fields:
        keys: List[ECPubKey]
        thresh_all: int
        keys_backup: List[ECPubKey]
        thresh_backup: int
        amount: Amount

    @unlock
    def _(self):
        return Threshold(self.thresh_all, self.keys)

    @unlock
    def backup_finish(self):
        return Threshold(self.thresh_backup, self.keys_backup) & Wait(Weeks(4))
