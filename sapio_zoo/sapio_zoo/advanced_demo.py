"""
advanced_demo.py
--------------------

This is an advanced contract which uses many features in Sapio.
"""
from typing import Optional, List, Tuple
from sapio_compiler import *


class DemoLayeredConditions(Contract):
    """
    DemoLayeredConditions is a example contract which demonstrates various
    features available in Sapio.
    """

    class Fields:
        key_a: PubKey
        key_b: PubKey
        key_c: PubKey
        key_d: PubKey
        amount: Amount
        setup: TransactionTemplate

    @require
    def a_signed(self) -> Clause:
        """
        Checks that a signature was attached from key_a
        """
        return SignedBy(self.key_a)

    @require
    def two_weeks(self) -> Clause:
        return Wait(Weeks(2))

    @require
    def one_month(self) -> Clause:
        return Wait(Weeks(4))

    @require
    def b_signed(self) -> Clause:
        return SignedBy(self.key_b)

    @require
    def c_signed(self) -> Clause:
        return SignedBy(self.key_c)

    @threshold(3, [a_signed, b_signed, c_signed])
    @unlock
    def all_signed(self) -> Clause:
        return Satisfied()

    @threshold(2, [a_signed, b_signed, c_signed])
    @guarantee
    def setup_tx(self) -> TransactionTemplate:
        # maybe make some assertions about timing...
        t: TransactionTemplate = self.setup
        return t

    @a_signed
    @two_weeks
    @unlock
    def time_release(self) -> Clause:
        return Satisfied()

    @one_month
    @require
    def d_signed_and_one_month(self) -> Clause:
        return SignedBy(self.key_d)

    @d_signed_and_one_month
    @guarantee
    def setup_tx2(self) -> TransactionTemplate:
        # maybe make some assertions about timing...
        t: TransactionTemplate = self.setup
        return t

    @threshold(3, [a_signed, b_signed, c_signed])
    @unlock_but_suggest
    def cooperate_example(
        self, state: Optional[List[Tuple[Amount, str]]] = None,
    ) -> TransactionTemplate:
        if state is None:
            # Default example:
            return self.setup
        else:
            tx = TransactionTemplate()
            tx.add_output(
                self.amount, DemoContractClose(amount=self.amount, payments=state),
            )
            return tx


class DemoContractClose(Contract):
    class Fields:
        amount: Amount
        payments: List[Tuple[Amount, str]]

    @require
    def wait(self):
        return Wait(Weeks(2))

    @wait
    @guarantee
    def make_payments(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        for (amt, to) in self.payments:
            tx.add_output(amt, PayToSegwitAddress(amount=amt, address=to))
        return tx
