from sapio.bitcoinlib.static_types import Amount, PubKey
from sapio.contract import Contract, guarantee, TransactionTemplate, unlock
from sapio.script.clause import RelativeTimeSpec, SignatureCheckClause


class Channel(Contract):
    class Fields:
        initial: Contract
        mpc_key: PubKey
        timeout: RelativeTimeSpec
        amount: Amount

    @guarantee
    def begin_contest(self):
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value,
                      ContestedChannel(amount=self.amount, initial=self.initial, timeout=self.timeout,
                                       mpc_key=self.mpc_key))
        return tx

    @unlock
    def cooperate(self):
        return SignatureCheckClause(self.mpc_key)


class ContestedChannel(Contract):
    class Fields:
        initial: Contract
        mpc_key: PubKey
        timeout: RelativeTimeSpec
        amount: Amount

    @guarantee
    def finish_contest(self):
        tx = TransactionTemplate()
        tx.set_sequence(self.timeout.assigned_value.time)
        tx.add_output(self.amount.assigned_value, self.initial.assigned_value)
        return tx
    @unlock
    def cooperate(self):
        return SignatureCheckClause(self.mpc_key)
