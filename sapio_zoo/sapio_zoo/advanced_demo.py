"""
advanced_demo.py
--------------------

This is an advanced contract which uses many features in Sapio.
"""
from typing import Optional, List, Tuple
from sapio_compiler import *


@contract
class DemoLayeredConditions:
    key_a: PubKey
    key_b: PubKey
    key_c: PubKey
    key_d: PubKey
    amount: Amount
    setup: TransactionTemplate


"""
DemoLayeredConditions is a example contract which demonstrates various
features available in Sapio.
"""

let = DemoLayeredConditions.let
finish = DemoLayeredConditions.finish
then = DemoLayeredConditions.then
threshold = DemoLayeredConditions.threshold
finish_or = DemoLayeredConditions.finish_or


@let
def a_signed(self) -> Clause:
    """
    Checks that a signature was attached from key_a
    """
    return SignedBy(self.key_a)


@let
def two_weeks(self) -> Clause:
    return Wait(Weeks(2))


@let
def one_month(self) -> Clause:
    return Wait(Weeks(4))


@let
def b_signed(self) -> Clause:
    return SignedBy(self.key_b)


@let
def c_signed(self) -> Clause:
    return SignedBy(self.key_c)


@threshold(3, [a_signed, b_signed, c_signed])
@finish
def all_signed(self) -> Clause:
    return Satisfied()


@threshold(3, [a_signed, b_signed, c_signed])
@then
def setup_tx(self) -> TransactionTemplate:
    # maybe make some assertions about timing...
    t: TransactionTemplate = self.setup
    return t


@a_signed
@two_weeks
@finish
def time_release(self) -> Clause:
    return Satisfied()


@one_month
@let
def d_signed_and_one_month(self) -> Clause:
    return SignedBy(self.key_d)


@d_signed_and_one_month
@then
def setup_tx2(self) -> TransactionTemplate:
    # maybe make some assertions about timing...
    t: TransactionTemplate = self.setup
    return t


@threshold(3, [a_signed, b_signed, c_signed])
@finish_or
def cooperate_example(
    self,
    state: Optional[List[Tuple[Amount, str]]] = None,
) -> TransactionTemplate:
    if state is None:
        # Default example:
        return self.setup
    else:
        tx = TransactionTemplate()
        tx.add_output(
            self.amount,
            DemoContractClose(amount=self.amount, payments=state),
        )
        return tx


@contract
class DemoContractClose:
    amount: Amount
    payments: List[Tuple[Amount, str]]


@DemoContractClose.let
def wait(self):
    return Wait(Weeks(2))


@wait
@DemoContractClose.then
def make_payments(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    for (amt, to) in self.payments:
        tx.add_output(amt, PayToSegwitAddress(amount=amt, address=to))
    return tx
