from txlang.bitcoinlib.static_types import PubKey, Amount
from txlang import Contract, unlock
from txlang.script_lang import SignatureCheckClause


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass

