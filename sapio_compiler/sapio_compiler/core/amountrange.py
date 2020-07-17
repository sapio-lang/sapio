from __future__ import annotations
from sapio_bitcoinlib.static_types import Amount
from typing import Final


class AmountRange:
    """
    Utility class which tracks the amount of funds that a contract has a
    guaranteed path to spend minimally and maximally.
    """

    MIN: Final[Amount] = Amount(0)
    """Minimum amount of BTC to send"""
    MAX: Final[Amount] = Amount(21_000_000 * 100_000_000)
    """Maximum amount of BTC to send"""

    def __init__(self) -> None:
        """
        By default we construct it with the max value for min, and the min
        value for max. This means that any subsequent update will be correct.
        """
        self.min = AmountRange.MAX
        self.max = AmountRange.MIN

    @staticmethod
    def of(a: Amount) -> AmountRange:
        ar = AmountRange()
        ar.update_range(a)
        return ar

    def get_min(self) -> Amount:
        return self.min

    def get_max(self) -> Amount:
        return self.max

    def update_range(self, amount: Amount) -> None:
        if not AmountRange.MIN <= amount <= AmountRange.MAX:
            raise ValueError("Invalid Amount of Bitcoin", amount)
        self.min = min(self.min, amount)
        self.max = max(self.max, amount)
