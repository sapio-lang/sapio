from sapio_compiler import *
from sapio_bitcoinlib.key import ECPubKey


class P2PK(Contract):
    class Fields:
        key: ECPubKey

    @unlock
    def spend(self) -> Clause:
        return SignedBy(self.key)
