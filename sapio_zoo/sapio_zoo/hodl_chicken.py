"""
This License applies solely to the file hodl_chicken.py.

Copyright (c) 2020, Pyskell and Judica, Inc
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:
    * Redistributions of source code must retain the above copyright
      notice, this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above copyright
      notice, this list of conditions and the following disclaimer in the
      documentation and/or other materials provided with the distribution.
    * Neither the name of the <organization> nor the
      names of its contributors may be used to endorse or promote products
      derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL <COPYRIGHT HOLDER> BE LIABLE FOR ANY
DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"""
from typing import Tuple, Callable

from bitcoin_script_compiler import Satisfied
from sapio_bitcoinlib.messages import COutPoint
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_compiler import contract, TransactionTemplate, SignedBy, Contract


@contract
class HodlChicken:
    alice_contract: Callable[[Amount], Contract]
    bob_contract: Callable[[Amount], Contract]
    alice_key: PubKey
    bob_key: PubKey
    alice_deposit: Amount
    bob_deposit: Amount
    winner_gets: Amount
    chicken_gets: Amount

@HodlChicken.require
def amounts_sum_correctly(self):
    # Make sure all sats will be spent when the game completes
    return (self.alice_deposit + self.bob_deposit) == self.winner_gets + self.chicken_gets

@HodlChicken.require
def equal_amounts(self):
    # Both participants should commit the same amount
    return self.alice_deposit == self.bob_deposit

@HodlChicken.let
def alice_is_a_chicken(self):
    return SignedBy(self.alice_key)

@alice_is_a_chicken
@HodlChicken.then
def alice_redeem(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.winner_gets, self.bob_contract(self.winner_gets))
    tx.add_output(self.chicken_gets, self.alice_contract(self.chicken_gets))
    return tx

@HodlChicken.let
def bob_is_a_chicken(self):
    return SignedBy(self.bob_key)

@bob_is_a_chicken
@HodlChicken.then
def bob_redeem(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.winner_gets, self.alice_contract(self.winner_gets))
    tx.add_output(self.chicken_gets, self.bob_contract(self.chicken_gets))
    return tx
