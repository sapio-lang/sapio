from sapio.bitcoinlib.static_types import Amount, PubKey
from sapio.contract import Contract, path, TransactionTemplate, unlock
from sapio.spending_conditions.script_lang import RelativeTimeSpec, SignatureCheckClause


class Channel(Contract):
    class Fields:
        initial: Contract
        mpc_key: PubKey
        timeout: RelativeTimeSpec
        amount: Amount

    @path
    def begin_contest(self):
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value,
                      ContestedChannel(amount=self.amount, initial=self.initial, timeout=self.timeout,
                                       mpc_key=self.mpc_key))
        return tx

    @unlock(lambda self: SignatureCheckClause(self.mpc_key))
    def cooperate(self): pass


class ContestedChannel(Contract):
    class Fields:
        initial: Contract
        mpc_key: PubKey
        timeout: RelativeTimeSpec
        amount: Amount

    @path
    def finish_contest(self):
        tx = TransactionTemplate()
        tx.set_sequence(self.timeout.assigned_value.time)
        tx.add_output(self.amount.assigned_value, self.initial.assigned_value)
        return tx

    @unlock(lambda self: SignatureCheckClause(self.mpc_key))
    def cooperate(self): pass
