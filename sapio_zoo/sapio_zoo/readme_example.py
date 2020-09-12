from typing import Tuple

from bitcoin_script_compiler import Days, SignedBy
from sapio_bitcoinlib.messages import COutPoint
from sapio_bitcoinlib.static_types import Amount, Bitcoin, PubKey, Sats
from sapio_compiler import Contract, TransactionTemplate, contract


@contract
class PayToPublicKey:
    key: PubKey


@PayToPublicKey.finish
def with_key(self):
    return SignedBy(self.key)


@contract
class BasicEscrow:
    alice: PubKey
    bob: PubKey
    escrow: PubKey


@BasicEscrow.finish
def redeem(self):
    return SignedBy(self.escrow) & (SignedBy(self.alice) | SignedBy(self.bob)) | (
        SignedBy(self.alice) & SignedBy(self.bob)
    )


@contract
class BasicEscrow2:
    alice: PubKey
    bob: PubKey
    escrow: PubKey


@BasicEscrow2.finish
def use_escrow(self):
    return SignedBy(self.escrow) & (SignedBy(self.alice) | SignedBy(self.bob))


@BasicEscrow2.finish
def cooperate_(self):
    return SignedBy(self.alice) & SignedBy(self.bob)


@contract
class TrustlessEscrow:
    alice: PubKey
    bob: PubKey
    alice_escrow: Tuple[Amount, Contract]
    bob_escrow: Tuple[Amount, Contract]


@TrustlessEscrow.then
def use_escrow_(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(*self.alice_escrow)
    tx.add_output(*self.bob_escrow)
    tx.set_sequence(Days(10))
    return tx


@TrustlessEscrow.finish
def cooperate(self):
    return SignedBy(self.alice) & SignedBy(self.bob)


if __name__ == "__main__":
    key_alice = b"0" * 32
    key_bob = b"1" * 32
    t = TrustlessEscrow(
        TrustlessEscrow.Props(
            alice=key_alice,
            bob=key_bob,
            alice_escrow=(
                Bitcoin(1),
                PayToPublicKey(PayToPublicKey.Props(key=key_alice)),
            ),
            bob_escrow=(Sats(10000), PayToPublicKey(PayToPublicKey.Props(key=key_bob))),
        )
    )

    t1 = TrustlessEscrow(
        TrustlessEscrow.Props(
            alice=key_alice,
            bob=key_bob,
            alice_escrow=(
                Bitcoin(1),
                PayToPublicKey(PayToPublicKey.Props(key=key_alice)),
            ),
            bob_escrow=(Sats(10000), PayToPublicKey(PayToPublicKey.Props(key=key_bob))),
        )
    )
    t2 = TrustlessEscrow(
        TrustlessEscrow.Props(
            alice=key_alice,
            bob=key_bob,
            alice_escrow=(
                Bitcoin(1),
                PayToPublicKey(PayToPublicKey.Props(key=key_alice)),
            ),
            bob_escrow=(Sats(10000) + Bitcoin(1), t1),
        )
    )
    print(t2.bind(COutPoint()))
    print(t2.witness_manager.get_p2wsh_script())
    print(t2.amount_range[1] / 100e6, t2.witness_manager.get_p2wsh_address())

    # t3 throws an error because we would lose value
    try:
        t3 = TrustlessEscrow(
            TrustlessEscrow.Props(
                alice=key_alice,
                bob=key_bob,
                alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
                bob_escrow=(Sats(10000), t1),
            )
        )
    except ValueError:
        pass
