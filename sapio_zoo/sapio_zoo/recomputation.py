from __future__ import annotations
import functools
from typing import List, Tuple, Callable, Protocol

from bitcoin_script_compiler import (
    Wait,
    SignedBy,
    Weeks,
)
from sapio_bitcoinlib.static_types import Amount, PubKey
from sapio_bitcoinlib.key import ECKey
from sapio_compiler import Contract, TransactionTemplate, contract
from dataclasses import field


@contract
class Recomputation:
    """
    Recomputation demonstrates how to use __post_init__ with sub-contracts to
    drive an adjusting computation and search a parameter space for a solution
    that meets constraints.
    """

    k1: PubKey
    k2: PubKey
    amount: Amount
    a: PostHook = field(init=False)
    b: PostHook = field(init=False)

    def __post_init__(self):
        var1 = 20
        var2 = 0
        not_ready = True

        # This Produces a trace which tries the following:
        # 20 != 2*0 --> Fails
        # 18 != 2*1 --> Fails
        # 16 != 2*2 --> Fails
        # 14 != 2*3 --> Fails
        # 12 != 2*4 --> Fails
        # 10 == 2*5 --> Succeeds
        #
        # This is clearly a dumb principle, but it shows
        # how you can write custom logic that enforces
        # relationships by passing data up the stack
        # from farther-down contract
        def hook(x: int):
            nonlocal not_ready
            if self.a.data.timeout() == x:
                not_ready = False

        while not_ready:
            self.a = PostHook.create(
                k=self.k1, timeout_=var1, mult=1, post_hook=lambda x: None
            )
            self.b = PostHook.create(k=self.k2, timeout_=var2, mult=2, post_hook=hook)
            var1 -= 2
            var2 += 1


# Checks that __post_init__ was correct.
@Recomputation.require
def check_timing(self):
    return self.a.data.timeout() == self.b.data.timeout()


@Recomputation.then
def a(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.amount, self.a)
    return tx


@Recomputation.then
def b(self) -> TransactionTemplate:
    tx = TransactionTemplate()
    tx.add_output(self.amount, self.b)
    return tx


@contract
class PostHook:
    k: PubKey
    timeout_: int
    mult: int
    post_hook: Callable[[int], None]

    def timeout(self) -> int:
        return self.timeout_ * self.mult

    def __post_init__(self):
        self.post_hook(self.timeout())


@PostHook.finish
def signed(self):
    return SignedBy(self.k) & Wait(Weeks(self.timeout()))
