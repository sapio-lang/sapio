from typing import Tuple

from bitcoin_script_compiler import Days, SignedBy
from sapio_bitcoinlib.messages import COutPoint
from sapio_bitcoinlib.static_types import Amount, Bitcoin, PubKey, Sats
from sapio_compiler import Contract, TransactionTemplate, guarantee, unlock


class PayToPublicKey(Contract):
    class Fields:
        key: PubKey

    @unlock
    def with_key(self):
        return SignedBy(self.key)


class BasicEscrow(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        escrow: PubKey

    @unlock
    def redeem(self):
        return SignedBy(self.escrow) & (SignedBy(self.alice) | SignedBy(self.bob)) | (
            SignedBy(self.alice) & SignedBy(self.bob)
        )


class BasicEscrow2(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        escrow: PubKey

    @unlock
    def use_escrow(self):
        return SignedBy(self.escrow) & (SignedBy(self.alice) | SignedBy(self.bob))

    @unlock
    def cooperate(self):
        return SignedBy(self.alice) & SignedBy(self.bob)


class TrustlessEscrow(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        alice_escrow: Tuple[Amount, Contract]
        bob_escrow: Tuple[Amount, Contract]

    @guarantee
    def use_escrow(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(*self.alice_escrow)
        tx.add_output(*self.bob_escrow)
        tx.set_sequence(Days(10))
        return tx

    @unlock
    def cooperate(self):
        return SignedBy(self.alice) & SignedBy(self.bob)


if __name__ == "__main__":
    key_alice = b"0" * 32
    key_bob = b"1" * 32
    t = TrustlessEscrow(
        alice=key_alice,
        bob=key_bob,
        alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
        bob_escrow=(Sats(10000), PayToPublicKey(key=key_bob)),
    )

    t1 = TrustlessEscrow(
        alice=key_alice,
        bob=key_bob,
        alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
        bob_escrow=(Sats(10000), PayToPublicKey(key=key_bob)),
    )
    t2 = TrustlessEscrow(
        alice=key_alice,
        bob=key_bob,
        alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
        bob_escrow=(Sats(10000) + Bitcoin(1), t1),
    )
    print(t2.bind(COutPoint()))
    print(t2.witness_manager.get_p2wsh_script())
    print(t2.amount_range[1] / 100e6, t2.witness_manager.get_p2wsh_address())

    # t3 throws an error because we would lose value
    try:
        t3 = TrustlessEscrow(
            alice=key_alice,
            bob=key_bob,
            alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
            bob_escrow=(Sats(10000), t1),
        )
    except ValueError:
        pass
