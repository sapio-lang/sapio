from __future__ import annotations
import sapio_compiler
import sapio_compiler.core.bindable_contract as bc
from bitcoinlib.static_types import Amount, Sats


class WithinFee:
    fee_modifier : Amount = Sats(100)

    def __init__(self, contract: bc.BindableContract, b: Amount) -> None:
        if contract.amount_range[0] + self.fee_modifier < b:
            raise ValueError(f"Contract {bc.__name__} May Burn Funds! Spent {contract.amount_range[0]} to {contract.amount_range[1]}, not within {b+self.fee_modifier}")

    @classmethod
    def change_fee_modifier(cls, fee_modifier:Amount):
        cls.fee_modifier = fee_modifier


class HasEnoughFunds:
    def __init__(self, contract: sapio.contract.bindable_contract.BindableContract, b: Amount) -> None:
        if contract.amount_range[1] > b:
            raise ValueError("Insufficient Funds", "Contract May Burn Funds!", contract, contract.amount_range, b)
