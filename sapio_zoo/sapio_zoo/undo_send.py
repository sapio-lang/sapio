from bitcoin_script_compiler import *
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, TransactionTemplate, contract


@contract
class UndoSend:
    from_contract: Contract
    to_key: PubKey
    amount: Amount
    timeout: TimeSpec

@UndoSend.let
def is_matured(self):
    return Wait(self.timeout)

@UndoSend.let
def check_key(self):
    return SignedBy(self.to_key)

@is_matured
@check_key
@UndoSend.finish
def complete(self):
    return Satisfied()

@UndoSend.then
def undo(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.amount, self.from_contract)
    return tx


@contract
class UndoSend2:
    from_contract: Contract
    to_contract: Contract
    amount: Amount
    timeout: TimeSpec

    class MetaData:
        color: str = "red"
        label: str = "UndoSend2"
    metadata: MetaData = MetaData()

@UndoSend2.then
def complete(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.set_sequence(self.timeout)
    tx.add_output(self.amount, self.to_contract)
    return tx

@UndoSend2.then
def undo(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.amount, self.from_contract)
    return tx
