from src.lib.bitcoinlib.static_types import PubKey, Amount
from src.lib.contract import Contract, unlock
from src.lib.script_lang import SignatureCheckClause


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass

