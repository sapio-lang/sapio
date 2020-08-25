from typing import Tuple, Callable

from bitcoin_script_compiler import Satisfied
from sapio_bitcoinlib.messages import COutPoint
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import Contract, TransactionTemplate, SignedBy, guarantee, check, require


class HodlChicken(Contract):
    class Fields:
        alice_contract: Callable[[Amount], Contract]
        bob_contract: Callable[[Amount], Contract]
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

    @require
    def alice_is_a_chicken(self):
        return SignedBy(self.alice_key)
    
    @alice_is_a_chicken
    @guarantee
    def alice_redeem(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.winner_gets, self.bob_contract(self.winner_gets))        
        tx.add_output(self.chicken_gets, self.alice_contract(self.chicken_gets))
        return tx

    @require
    def bob_is_a_chicken(self):
        return SignedBy(self.bob_key)
    
    @bob_is_a_chicken
    @guarantee
    def bob_redeem(self) -> TransactionTemplate:
        tx = TransactionTemplate()
        tx.add_output(self.winner_gets, self.alice_contract(self.winner_gets))        
        tx.add_output(self.chicken_gets, self.bob_contract(self.chicken_gets))
        return tx  