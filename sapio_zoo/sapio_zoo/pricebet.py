from __future__ import annotations
from typing import (
    Dict,
    Generic,
    List,
    Literal,
    Optional,
    Protocol,
    Tuple,
    Type,
    TypeVar,
    Union,
)

from bitcoinlib.static_types import Amount, Hash, PubKey
from sapio_compiler.contract.core.bindable_contract import BindableContract
from sapio_compiler.contract.contract import Contract
from sapio_compiler.contract.decorators import (
    check,
    enable_if,
    guarantee,
    require,
    unlock,
    unlock_but_suggest,
)
from sapio_compiler.contract.core.txtemplate import TransactionTemplate
from sapio_zoo.p2pk import PayToPubKey, PayToSegwitAddress
from bitcoin_script_compiler.clause import (
    Clause,
    PreImageCheckClause,
    RelativeTimeSpec,
    SatisfiedClause,
    SignatureCheckClause,
)
from bitcoin_script_compiler.variable import AssignedVariable


T1 = TypeVar("T1")
T2 = TypeVar("T2")


def BinaryBetFactory(t1: Type[T1], t2: Type[T2]):
    class BinaryBet(Contract):
        class Fields:
            price: int
            h_price_hi: Hash  # preimage revealed if price above threshold
            h_price_lo: Hash  # preimage revealed if price below threshold
            amount: Amount
            hi_outcome: T1
            lo_outcome: T2

        class MetaData:
            label = lambda self: f"BinaryOption[price > ${self.price.assigned_value}]"
            color = lambda self: "turquoise"

        @require
        def price_hi(self):
            return PreImageCheckClause(self.h_price_hi)

        @require
        def price_lo(self):
            return PreImageCheckClause(self.h_price_lo)

        if t1 is PubKey:

            @price_hi
            @unlock
            def pay_hi(self):
                return SignatureCheckClause(self.hi_outcome)

        elif t1 is Contract:

            @price_hi
            @guarantee
            def pay_hi(self):
                tx = TransactionTemplate()
                tx.add_output(
                    self.amount.assigned_value, self.hi_outcome.assigned_value
                )
                return tx

        if t2 is PubKey:

            @price_lo
            @unlock
            def pay_lo(self):
                return SignatureCheckClause(self.lo_outcome)

        elif t2 is Contract:

            @price_lo
            @guarantee
            def pay_lo(self):
                tx = TransactionTemplate()
                tx.add_output(
                    self.amount.assigned_value, self.lo_outcome.assigned_value
                )
                return tx

    return BinaryBet


b = BinaryBetFactory(Contract, Contract)


class PriceOracle:
    class BetStructure:
        price_array: List[Tuple[int, Tuple[Hash, Hash], Contract]]

        def __init__(self, l: List[Tuple[int, Tuple[Hash, Hash], Contract]]):
            self.price_array = l

        @classmethod
        def from_json_data(
            cls, data: List[Tuple[int, Tuple[Hash, Hash], Tuple[Amount, str]]], ctx
        ):
            pass


    class Fields:
        price_array: PriceOracle.BetStructure
        amount: Amount

    @staticmethod
    def generate(
        bets: BetStructure, amount: Amount, is_sorted: bool = False
    ) -> BinaryBet:
        price_array = bets.price_array
        if len(price_array) > 1:
            if not is_sorted:
                if any(
                    price_array[i][0] < price_array[i + 1][0]
                    for i in range(len(price_array) - 1)
                ):
                    price_array.sort()
                    price_array = price_array[::-1]

            middle = len(price_array) // 2
            price, (h_lo, h_hi), _ = price_array[:middle][-1]

            lo_outcome = PriceOracle.generate(
                PriceOracle.BetStructure(price_array[middle:]), amount, True
            )
            hi_outcome = PriceOracle.generate(
                PriceOracle.BetStructure(price_array[:middle]), amount, True
            )
            return b(
                price=price,
                hi_outcome=hi_outcome,
                lo_outcome=lo_outcome,
                h_price_hi=h_hi,
                h_price_lo=h_lo,
                amount=amount,
            )
        else:
            assert len(price_array)
            return price_array[0][-1]
