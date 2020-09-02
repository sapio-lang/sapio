from sapio_compiler import *
from sapio_bitcoinlib.key import ECPubKey


@contract
class P2PK:
    key: ECPubKey
    amount: Amount


@P2PK.finish
def spend(self) -> Clause:
    return SignedBy(self.key)
