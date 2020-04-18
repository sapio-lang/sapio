from sapio.bitcoinlib.static_types import PubKey, Amount
from sapio.contract import Contract
from sapio.decorators import unlock, pay_address
from sapio.script.clause import SignatureCheckClause


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass


class PayToSegwitAddress(Contract):
    class Fields:
        amount: Amount
        address: str

    class MetaData:
        color = lambda self: "grey"
        label = lambda self: "Segwit Address"

    @pay_address
    def _(self):
        return (self.amount.assigned_value, self.address.assigned_value)
