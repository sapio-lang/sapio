from sapio.examples.undo_send import UndoSend, UndoSend2
from sapio.bitcoinlib.static_types import Amount
from sapio.spending_conditions.script_lang import TimeSpec
from sapio.contract import Contract, TransactionTemplate, path


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
        tx.set_sequence(self.timeout.assigned_value.time)
        tx.add_output(self.amount_step.assigned_value,
                      UndoSend(from_contract=self.cold_storage,
                               to_key=self.hot_storage,
                               timeout=self.mature,
                               amount=self.amount_step))
        if self.n_steps.assigned_value > 1:
            steps_left = self.n_steps.assigned_value - 1
            sub_amount = (self.n_steps.assigned_value - 1) * self.amount_step.assigned_value
            sub_vault = Vault(cold_storage=self.cold_storage,
                              hot_storage=self.hot_storage,
                              n_steps=self.n_steps.assigned_value - 1,
                              timeout=self.timeout,
                              mature=self.mature,
                              amount_step=self.amount_step)
            tx.add_output(sub_amount, sub_vault)
        return tx

    @path
    def to_cold(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.n_steps.assigned_value * self.amount_step.assigned_value,
                      self.cold_storage.assigned_value)
        return tx

class Vault2(Contract):
    class Fields:
        cold_storage: Contract
        hot_storage: Contract
        n_steps: int
        amount_step: Amount
        timeout: TimeSpec
        mature: TimeSpec


    class MetaData:
        color = lambda self: "blue"
        label = lambda self: "Vault"

    @path
    def step(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.set_sequence(self.timeout.assigned_value.time)
        tx.add_output(self.amount_step.assigned_value,
                      UndoSend2(from_contract=self.cold_storage,
                               to_contract=self.hot_storage,
                               timeout=self.mature,
                               amount=self.amount_step))
        if self.n_steps.assigned_value > 1:
            steps_left = self.n_steps.assigned_value - 1
            sub_amount = (self.n_steps.assigned_value - 1) * self.amount_step.assigned_value
            sub_vault = Vault2(cold_storage=self.cold_storage,
                              hot_storage=self.hot_storage,
                              n_steps=self.n_steps.assigned_value - 1,
                              timeout=self.timeout,
                              mature=self.mature,
                              amount_step=self.amount_step)
            tx.add_output(sub_amount, sub_vault)
        return tx

    @path
    def to_cold(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.n_steps.assigned_value * self.amount_step.assigned_value,
                      self.cold_storage.assigned_value)
        return tx
