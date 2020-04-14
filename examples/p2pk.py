from sapio.bitcoinlib.static_types import PubKey, Amount
from sapio import Contract, unlock
from sapio.script_lang import SignatureCheckClause


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass

