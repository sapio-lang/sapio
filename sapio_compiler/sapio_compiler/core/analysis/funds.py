from __future__ import annotations

import sapio_compiler
import sapio_compiler.core.bindable_contract as bc
from sapio_bitcoinlib.static_types import Amount, Sats
from typing import Any


class WithinFee:
    fee_modifier: Amount = Sats(100)

    def __init__(
        self, contract: bc.BindableContractProtocol, amount_sent: Amount
    ) -> None:
        if contract.amount_range.min + self.fee_modifier < amount_sent:
            raise ValueError(
                f"Contract {bc.__class__.__name__} May Burn Funds!",
                f"Spent {contract.amount_range.min} to {contract.amount_range.max}, not within {amount_sent+self.fee_modifier}",
            )

    @classmethod
    def change_fee_modifier(cls, fee_modifier: Amount) -> None:
        cls.fee_modifier = fee_modifier


class HasEnoughFunds:
    def __init__(
        self,
        contract: sapio_compiler.core.bindable_contract.BindableContractProtocol,
        amount_sent: Amount,
    ) -> None:
        if contract.amount_range.max > amount_sent:
            raise ValueError(
                f"Contract {contract.__class__.__name__} May Burn Funds!",
                f"Insufficient Funds sent, {contract.amount_range.max} more than {amount_sent}",
            )
