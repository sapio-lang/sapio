from src.lib.contract import Contract, Amount, TimeSpec, TransactionTemplate, PubKey, AfterClause, SignatureCheckClause, \
    unlock, path


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
        tx.add_output(self.amount.value, self.from_contract.value)
        return tx