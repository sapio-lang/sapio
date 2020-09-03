from __future__ import annotations

from itertools import combinations
from typing import Optional

from sapio_bitcoinlib.key import ECPubKey
from sapio_compiler import (
    AbsoluteTimeSpec,
    Amount,
    Contract,
    PubKey,
    RelativeTimeSpec,
    SignedBy,
    Threshold,
    TimeSpec,
    TransactionTemplate,
    Wait,
    Weeks,
    contract,
)
from sapio_stdlib.p2pk import P2PK


@contract
class Capitulation:
    base: Amount
    premium: Amount
    unlock_date: TimeSpec
    owner: PubKey
    keeper: PubKey


@Capitulation.let
def wait(self) -> Clause:
    return Wait(self.unlock_date) & SignedBy(self.owner)


@wait
@Capitulation.finish_or
def redeem(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    if isinstance(self.unlock_date, RelativeTimeSpec):
        tx.set_sequence(self.unlock_date.sequence)
    elif isinstance(self.unlock_date, AbsoluteTimeSpec):
        tx.set_lock_time(self.unlock_date.locktime)
    total = self.base + self.premium
    tx.add_output(total, P2PK.create(amount=total, key=self.owner))
    return tx


@Capitulation.let
def capitulate(self) -> Clause:
    return SignedBy(self.keeper) & SignedBy(self.owner)


@capitulate
@Capitulation.finish_or
def sell(self, price: Optional[Amount] = None) -> TransactionTemplate:
    price = self.premium if self.price is None else price
    tx = TransactionTemplate()
    tx.add_output(price, P2PK.create(amount=price, key=self.keeper))
    total = self.base + self.premium
    assert price < total
    total -= price
    tx.add_output(total, P2PK.create(amount=total, key=self.owner))
    tx.add_output(price, P2PK.create(amount=price, key=self.keeper))
    return tx
