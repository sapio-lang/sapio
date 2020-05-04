import os

import tornado

import sapio_zoo
import sapio_zoo.channel
import sapio_zoo.basic_vault
import sapio_zoo.p2pk
import sapio_zoo.subscription
from bitcoinlib import segwit_addr
from sapio_zoo.tree_pay import TreePay
from sapio_zoo.undo_send import UndoSend2
from sapio_zoo.pricebet import PriceOracle
from bitcoin_script_compiler.clause import (
    AbsoluteTimeSpec,
    Days,
    RelativeTimeSpec,
    TimeSpec,
    Weeks,
)

from .ws import CompilerWebSocket
from bitcoinlib.static_types import Bitcoin, PubKey, Amount
from sapio_zoo.basic_vault import Vault
from sapio_zoo.p2pk import PayToPubKey
from sapio_zoo.smarter_vault import SmarterVault


def make_app():
    return tornado.web.Application([(r"/", CompilerWebSocket),], autoreload=True)


# example_to_make = "payroll"
example_to_make = "vault"
example_to_make = "Price Contract"
example_to_make = "tree"
if __name__ == "__main__":
    CompilerWebSocket.add_contract("Channel", sapio_zoo.channel.BasicChannel)
    CompilerWebSocket.add_contract("Pay to Public Key", sapio_zoo.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Vault", sapio_zoo.basic_vault.Vault2)
    CompilerWebSocket.add_contract("Subscription", sapio_zoo.subscription.auto_pay)
    CompilerWebSocket.add_contract("TreePay", TreePay)
    generate_n_address = [
        segwit_addr.encode("bcrt", 0, os.urandom(32)) for _ in range(64)
    ]
    def generate_address():
        return sapio_zoo.p2pk.PayToSegwitAddress(
            amount=0, address=segwit_addr.encode("bcrt", 0, os.urandom(32))
        )

    if example_to_make == "tree":
        payments = [
            (5, sapio_zoo.p2pk.PayToSegwitAddress(amount=0, address=address))
            for address in generate_n_address
        ]
        example = TreePay(payments=payments, radix=4)
        CompilerWebSocket.set_example(example)
    # amount: Amount
    # recipient: PayToSegwitAddress
    # schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
    # return_address: PayToSegwitAddress
    # watchtower_key: PubKey
    # return_timeout: RelativeTimeSpec

    if example_to_make == "payroll":
        N_EMPLOYEES = 4
        employee_addresses = [(1, generate_address()) for _ in range(N_EMPLOYEES)]

        import datetime

        now = datetime.datetime.now()
        day = datetime.timedelta(1)
        DURATION = 3
        employee_payments = [
            (
                perdiem * DURATION,
                sapio_zoo.subscription.CancellableSubscription(
                    amount=perdiem * DURATION,
                    recipient=address,
                    schedule=[
                        (AbsoluteTimeSpec.from_date(now + (1 + x) * day), perdiem)
                        for x in range(DURATION)
                    ],
                    return_address=generate_address(),
                    watchtower_key=b"",
                    return_timeout=Days(1),
                ),
            )
            for (perdiem, address) in employee_addresses
        ]
        tree1 = TreePay(payments=employee_payments, radix=2)
        sum_pay = [((amt * DURATION), addr) for (amt, addr) in employee_addresses]
        tree2 = TreePay(payments=sum_pay, radix=2)
        total_amount = sum(x for (x, _) in sum_pay)
        example2 = UndoSend2(
            to_contract=tree2,
            from_contract=tree1,
            amount=total_amount,
            timeout=Days(10),
        )

        CompilerWebSocket.set_example(example2)
    if example_to_make == "Price Contract":
        N_TIERS = 16
        price_tiers = [(1, generate_address()) for _ in range(N_TIERS)]
        bet = PriceOracle.generate(
            bets=PriceOracle.BetStructure(
                [
                    (idx + 10, (b"a", b"b"), tier[1])
                    for (idx, tier) in enumerate(price_tiers)
                ]
            ),
            amount=1,
        )
        example2 = UndoSend2(
            to_contract=bet,
            from_contract=generate_address(),
            amount=1,
            timeout=Days(10),
        )
        CompilerWebSocket.set_example(example2)
    from functools import lru_cache

    if example_to_make == "vault":
        key2 = generate_address()

        @lru_cache()
        def cold_storage(v: Amount):
            # TODO: Use a real PubKey Generator
            payments = [
                (v // 10, PayToPubKey(key=os.urandom(32), amount=v // 10))
                for _ in range(10)
            ]
            return TreePay(payments=payments, radix=4)

        s = SmarterVault(
            cold_storage=cold_storage,
            hot_storage=key2,
            n_steps=5,
            timeout=Weeks(1),
            mature=Weeks(2),
            amount_step=Bitcoin(100),
        )

        CompilerWebSocket.set_example(s)

    print(CompilerWebSocket.example_message)

    app = make_app()
    app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
