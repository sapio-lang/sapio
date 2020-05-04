from bitcoin_script_compiler import SignatureCheckClause
from bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, pay_address, unlock


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
