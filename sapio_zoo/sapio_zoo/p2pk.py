from bitcoin_script_compiler import SignedBy
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, pay_address, unlock, AmountRange


class PayToPubKey(Contract):
    class Fields:
        key: PubKey
        amount: Amount

    @unlock
    def with_key(self):
        return SignedBy(self.key)


class PayToSegwitAddress(Contract):
    """
    Allows inputting an external opaque segwit address.

    The amount argument should be by default set to the amount being sent to
    that address. This sets the min/max values on the amount range.
    """

    class Fields:
        amount: AmountRange
        address: str

    class MetaData:
        def color(self):
            return "grey"

        def label(self):
            return "Segwit Address"

    @pay_address
    def _(self):
        return (self.amount, self.address)
