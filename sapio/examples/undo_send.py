from sapio.contract import Contract, TransactionTemplate, unlock, path
from sapio.script_lang import *


class UndoSend(Contract):
    class Fields:
        from_contract: Contract
        to_key: PubKey
        amount: Amount
        timeout: TimeSpec

    @unlock(lambda self: AfterClause(self.timeout)*SignatureCheckClause(self.to_key))
    def _(self): pass

    @path(lambda self: SignatureCheckClause(self.to_key))
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value, self.from_contract.assigned_value)
        return tx