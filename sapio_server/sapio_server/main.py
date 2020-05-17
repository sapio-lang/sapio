import os

import tornado

import sapio_zoo
import sapio_zoo.channel
import sapio_zoo.p2pk
import sapio_zoo.subscription
from bitcoinlib import segwit_addr
from sapio_zoo.tree_pay import TreePay
from sapio_zoo.undo_send import UndoSend2
from sapio_zoo.pricebet import PriceOracle
from sapio_compiler import (
    AbsoluteTimeSpec,
    Days,
    RelativeTimeSpec,
    TimeSpec,
    Weeks,
    AmountRange,
)

from .ws import CompilerWebSocket
from bitcoinlib.static_types import Bitcoin, PubKey, Amount
from sapio_zoo.p2pk import PayToPubKey
from sapio_zoo.smarter_vault import SmarterVault


def make_app():
    return tornado.web.Application([(r"/", CompilerWebSocket),], autoreload=True)

if __name__ == "__main__":
    CompilerWebSocket.add_contract("Channel", sapio_zoo.channel.BasicChannel)
    CompilerWebSocket.add_contract("Pay to Public Key", sapio_zoo.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Subscription", sapio_zoo.subscription.auto_pay)
    CompilerWebSocket.add_contract("TreePay", TreePay)
    generate_n_address = [
        segwit_addr.encode("bcrt", 0, os.urandom(32)) for _ in range(64)
    ]
    payments = [
        (
            5,
            sapio_zoo.p2pk.PayToSegwitAddress(
                amount=AmountRange.of(0), address=address
            ),
        )
        for address in generate_n_address
    ]
    example = TreePay(payments=payments, radix=4)
    CompilerWebSocket.set_example(example)
    make_app = make_app()
    make_app.listen(8888)
    tornado.ioloop.IOLoop.current().start()