from sapio_compiler import *


class P2PK(Contract):
    class Fields:
        key: PubKey

    @unlock
    def spend(self) -> Clause:
        return SignedBy(self.key)
