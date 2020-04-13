from src.examples.undo_send import UndoSend
from src.lib.contract import Contract, Amount, TimeSpec, TransactionTemplate, path


class Vault(Contract):
    class Fields:
        cold_storage: Contract
        hot_storage: Contract
        n_steps: int
        amount_step: Amount
        timeout: TimeSpec
        mature: TimeSpec

    @path
    def step(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount_step.value,
                      UndoSend(from_contract=self.cold_storage,
                               to_key=self.hot_storage,
                               timeout=self.mature,
                               amount=self.amount_step))
        if self.n_steps.value > 1:
            steps_left = self.n_steps.value - 1
            sub_amount = (self.n_steps.value-1) * self.amount_step.value
            sub_vault = Vault(cold_storage=self.cold_storage,
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
        tx.add_output(self.n_steps.value * self.amount_step.value,
        self.cold_storage.value)
        return tx