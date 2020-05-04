from bitcoinlib.static_types import PubKey, Amount
from sapio.contract import Contract, unlock, pay_address
from sapio.script.clause import SignatureCheckClause


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock
    def with_key(self):
        return SignatureCheckClause(self.key)


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
