from typing import Callable

from src.examples.undo_send import UndoSend
from src.lib.bitcoinlib.static_types import Amount
from src.lib.contract import Contract, TransactionTemplate, path
from src.lib.script_lang import TimeSpec


class SmarterVault(Contract):
    class Fields:
        cold_storage: Callable[[Amount], Contract]
        hot_storage: Contract
        n_steps: int
        amount_step: Amount
        timeout: TimeSpec
        mature: TimeSpec

    @path
    def step(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount_step.value,
                      UndoSend(from_contract=self.cold_storage.value(self.amount_step.value),
                               to_key=self.hot_storage,
                               timeout=self.mature,
                               amount=self.amount_step))
        if self.n_steps.value > 1:
            steps_left = self.n_steps.value - 1
            sub_amount = (self.n_steps.value-1) * self.amount_step.value
            sub_vault = SmarterVault(cold_storage=self.cold_storage,
                                hot_storage=self.hot_storage,
                                n_steps=self.n_steps.value - 1,
                                timeout=self.timeout,
                                mature=self.mature,
                                amount_step=self.amount_step)
            tx.add_output(sub_amount, sub_vault)
        return tx

    @path
    def to_cold(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        value = self.n_steps.value * self.amount_step.value
        tx.add_output(value, self.cold_storage.value(value))
        return tx

