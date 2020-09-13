from typing import List, Tuple, Callable, Protocol

from bitcoin_script_compiler import (
    Wait,
    SignedBy,
    Weeks,
)
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_bitcoinlib.key import ECKey
from sapio_compiler import Contract, TransactionTemplate, contract
from dataclasses import field


class DataHasTimeout(Protocol):
    """
    DataHasTimeout requires a method which extracts a timeout.
    """
    def timeout(self) -> int:
        pass


class HasTimeout(Contract):
    """
    HasTimeout is a protocol for a contract which requires that the data field
    must be of type DataHasTimeout
    """
    data: DataHasTimeout


@contract
class ChecksSubCondition:
    """
    ChecksSubCondition demonstrates a contract which verifies the compatibility
    of trait data between two sub-contracts.
    """
    a_f: Callable[[Amount], HasTimeout]
    b_f: Callable[[Amount], HasTimeout]
    c: Amount
    a: HasTimeout = field(init=False)
    b: HasTimeout = field(init=False)

    def __post_init__(self):
        self.a = self.a_f(self.c)
        self.b = self.b_f(self.c)


@ChecksSubCondition.require
def check_timing(self):
    return self.a.data.timeout() > self.b.data.timeout()


@ChecksSubCondition.then
def a(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.c, self.a)
    return tx


@ChecksSubCondition.then
def b(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.c, self.b)
    return tx


@contract
class SubCondition:
    k: PubKey
    timeout_: int

    def timeout(self) -> int:
        return self.timeout_


@SubCondition.finish
def signed(self):
    return SignedBy(self.k) & Wait(Weeks(self.timeout()))
