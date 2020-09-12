import os

import tornado

import sapio_zoo
import sapio_zoo.channel
import sapio_zoo.p2pk
import sapio_zoo.subscription
from sapio_bitcoinlib import segwit_addr
from sapio_bitcoinlib.key import ECKey
from sapio_bitcoinlib.static_types import Amount, Bitcoin, PubKey, Sats
from sapio_compiler import (AbsoluteTimeSpec, AmountRange, Days,
                            RelativeTimeSpec, TimeSpec, Weeks)
from sapio_zoo.hodl_chicken import HodlChicken
from sapio_zoo.p2pk import PayToPubKey, PayToSegwitAddress
from sapio_zoo.pricebet import PriceOracle
from sapio_zoo.smarter_vault import SmarterVault
from sapio_zoo.tree_pay import TreePay
from sapio_zoo.undo_send import UndoSend2

from .ws import CompilerWebSocket


def random_k():
    e = ECKey()
    e.generate()
    return e.get_pubkey()



def make_app():
    return tornado.web.Application([(r"/", CompilerWebSocket)], autoreload=True)


if __name__ == "__main__":
    CompilerWebSocket.add_contract("Channel", sapio_zoo.channel.BasicChannel)
    CompilerWebSocket.add_contract("Pay to Public Key", sapio_zoo.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Subscription", sapio_zoo.subscription.auto_pay)
    CompilerWebSocket.add_contract("TreePay", TreePay)
    CompilerWebSocket.add_contract("Chicken", HodlChicken)
    generate_n_address = [
        segwit_addr.encode("bcrt", 0, os.urandom(32)) for _ in range(64)
    ]
    payments = [
        (
            5,
            sapio_zoo.p2pk.PayToSegwitAddress.create(
                amount=AmountRange.of(0), address=address
            ),
        )
        for address in generate_n_address
    ]
    example = TreePay.create(payments=payments, radix=4)

    from sapio_bitcoinlib.address import key_to_p2pkh, key_to_p2wpkh, script_to_p2wsh
    from sapio_bitcoinlib.script import CScript
    alice_script = script_to_p2wsh(CScript([b"Alice's Key Goes Here!"]))
    bob_script = script_to_p2wsh(CScript([b"Bob's Key Goes Here!"]))
    alice_key = random_k()
    bob_key = random_k()

    hodl_chicken = HodlChicken.create(
        alice_contract=lambda x: PayToSegwitAddress.create(
            amount=AmountRange.of(x), address=alice_script
        ),
        bob_contract=lambda x: PayToSegwitAddress.create(
            amount=AmountRange.of(x), address=bob_script
        ),
        alice_key=alice_key,
        bob_key=bob_key,
        alice_deposit=Sats(100_000_000),
        bob_deposit=Sats(100_000_000),
        winner_gets=Sats(150_000_000),
        chicken_gets=Sats(50_000_000),
    )
    example = hodl_chicken
    CompilerWebSocket.set_example(example)
    make_app = make_app()
    make_app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
