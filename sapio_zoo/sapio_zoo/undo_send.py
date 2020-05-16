from bitcoin_script_compiler import *
from bitcoinlib.static_types import Amount
from sapio_compiler import Contract, TransactionTemplate, guarantee, require, unlock


class UndoSend(Contract):
    class Fields:
        from_contract: Contract
        to_key: PubKey
        amount: Amount
        timeout: TimeSpec

    @require
    def is_matured(self):
        return AfterClause(self.timeout)

    @require
    def check_key(self):
        return SignatureCheckClause(self.to_key)

    @is_matured
    @check_key
    @unlock
    def finish(self):
        return SatisfiedClause()

    @guarantee
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount, self.from_contract)
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
        tx.set_sequence(self.timeout)
        tx.add_output(self.amount, self.to_contract)
        return tx

    @guarantee
    def undo(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.amount, self.from_contract)
        return tx
