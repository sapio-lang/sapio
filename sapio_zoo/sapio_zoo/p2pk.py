from bitcoin_script_compiler import SignedBy
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, AmountRange, contract
from dataclasses import dataclass

from sapio_stdlib.p2pk import P2PK as PayToPubKey


@contract
class PayToSegwitAddress:
    """
    Allows inputting an external opaque segwit address.

    The amount argument should be by default set to the amount being sent to
    that address. This sets the min/max values on the amount range.
    """

    amount: AmountRange
    address: str

    @dataclass
    class MetaData:
        label = "Segwit Address"
        color = "grey"

    metadata: MetaData = MetaData()


def p(self):
    return (self.amount, self.address)


PayToSegwitAddress.override = p
