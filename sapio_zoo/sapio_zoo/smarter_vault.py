from typing import Callable

from bitcoin_script_compiler import TimeSpec
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, TransactionTemplate, contract
from sapio_zoo.undo_send import UndoSend, UndoSend2


@contract
class SmarterVault:
    cold_storage: Callable[[Amount], Contract]
    hot_storage: Contract
    n_steps: int
    amount_step: Amount
    timeout: TimeSpec
    mature: TimeSpec

    class MetaData:
        color: str = "blue"
        label: str = "Smart Vault"

    metadata: MetaData = MetaData()


@SmarterVault.then
def step(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.set_sequence(self.timeout)
    tx.add_output(
        self.amount_step,
        UndoSend2(
            UndoSend2.Props(
                from_contract=self.cold_storage(self.amount_step),
                to_contract=self.hot_storage,
                timeout=self.mature,
                amount=self.amount_step,
            )
        ),
    )
    if self.n_steps > 1:
        sub_amount = (self.n_steps - 1) * self.amount_step
        sub_vault = SmarterVault(
            SmarterVault.Props(
                cold_storage=self.cold_storage,
                hot_storage=self.hot_storage,
                n_steps=self.n_steps - 1,
                timeout=self.timeout,
                mature=self.mature,
                amount_step=self.amount_step,
            )
        )
        tx.add_output(sub_amount, sub_vault)
    return tx


@SmarterVault.then
def to_cold(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    value = self.n_steps * self.amount_step
    tx.add_output(value, self.cold_storage(value))
    return tx
