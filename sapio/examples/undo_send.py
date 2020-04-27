from sapio.bitcoinlib.static_types import Amount
from sapio.contract import Contract, TransactionTemplate
from sapio.contract.decorators import guarantee, require, unlock
from sapio.script.clause import *


class UndoSend(Contract):
    class Fields:
        from_contract: Contract
        to_key: PubKey
        amount: Amount
        timeout: TimeSpec

    @unlock(lambda self: AfterClause(self.timeout)&SignatureCheckClause(self.to_key))
    def _(self): pass

    @require(lambda self: SignatureCheckClause(self.to_key))
    @guarantee
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value, self.from_contract.assigned_value)
        return tx

class UndoSend2(Contract):
    class Fields:
        from_contract: Contract
        to_contract: Contract
        amount: Amount
        timeout: TimeSpec

    class MetaData:
        color = lambda self: "red"
        label = lambda self: "Undo Send"

    @guarantee
    def complete(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.set_sequence(self.timeout.assigned_value.time)
        tx.add_output(self.amount.assigned_value, self.to_contract.assigned_value)
        return tx

    @guarantee
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value, self.from_contract.assigned_value)
        return tx
