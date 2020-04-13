from src.lib.contract import Contract, PubKey, Amount, SignatureCheckClause, unlock


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass