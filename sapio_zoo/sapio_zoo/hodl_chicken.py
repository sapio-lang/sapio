from typing import Tuple

from bitcoin_script_compiler import Satisfied
from sapio_bitcoinlib.messages import COutPoint
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, TransactionTemplate, SignedBy, unlock, check
from sapio_stdlib.p2pk import P2PK


class HodlChicken(Contract):
    class Fields:
        alice_key: PubKey
        bob_key: PubKey
        alice_deposit: Amount
        bob_deposit: Amount
        winner_gets: Amount
        chicken_gets: Amount

    @check
    def amounts_sum_correctly(self):
        # Make sure all sats will be spent when the game completes
        return (self.alice_deposit + self.bob_deposit) == self.winner_gets + self.chicken_gets

    @check
    def equal_amounts(self):
        # Both participants should commit the same amount
        return self.alice_deposit == self.bob_deposit

    @unlock
    def alice_is_a_chicken(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.winner_gets, P2PK(key=self.bob_key))        
        tx.add_output(self.chicken_gets, P2PK(key=self.alice_key))
        return SignedBy(self.alice_key)

    @unlock
    def bob_is_a_chicken(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.winner_gets, P2PK(key=self.alice_key))        
        tx.add_output(self.chicken_gets, P2PK(key=self.bob_key))
        return SignedBy(self.bob_key)