import os

import tornado

import sapio
import sapio.examples.basic_vault
import sapio.examples.p2pk
import sapio.examples.subscription
from sapio.bitcoinlib import segwit_addr
from sapio.examples.tree_pay import TreePay
from sapio.examples.undo_send import UndoSend2
from sapio.script.clause import (AbsoluteTimeSpec, Days, RelativeTimeSpec,
                                 TimeSpec)

from .ws import CompilerWebSocket


def make_app():
    return tornado.web.Application([
        (r"/", CompilerWebSocket),
    ], autoreload=True)



if __name__ == "__main__":
    CompilerWebSocket.add_contract("Pay to Public Key", sapio.examples.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Vault", sapio.examples.basic_vault.Vault2)
    CompilerWebSocket.add_contract("Subscription", sapio.examples.subscription.auto_pay)
    CompilerWebSocket.add_contract("TreePay", TreePay)
    generate_n_address = [segwit_addr.encode('bcrt', 0, os.urandom(32)) for _ in range(16)]
    payments = [(5, sapio.examples.p2pk.PayToSegwitAddress(amount=0, address=address)) for address in
                generate_n_address]
    example = TreePay(payments=payments, radix=8)
    # amount: Amount
    # recipient: PayToSegwitAddress
    # schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
    # return_address: PayToSegwitAddress
    # watchtower_key: PubKey
    # return_timeout: RelativeTimeSpec

    N_EMPLOYEES = 2
    def generate_address():
        return sapio.examples.p2pk.PayToSegwitAddress(amount=0,
                                                      address=segwit_addr.encode('bcrt', 0, os.urandom( 32)))
    employee_addresses = [(1, generate_address()) for _ in range(N_EMPLOYEES)]

    import datetime

    now = datetime.datetime.now()
    day = datetime.timedelta(1)
    DURATION = 2
    employee_payments = [(perdiem * DURATION,
                          sapio.examples.subscription.CancellableSubscription(amount=perdiem * DURATION,
                                                                              recipient=address, schedule=[
                                  (AbsoluteTimeSpec.from_date(now + (1 + x) * day), perdiem) for x in range(DURATION)],
                                                                              return_address=generate_address(),
                                                                              watchtower_key=b"",
                                                                              return_timeout=Days(1))) for
                         (perdiem, address) in employee_addresses]
    tree1 = TreePay(payments=employee_payments, radix=2)
    sum_pay = [((amt*DURATION),addr) for (amt, addr) in employee_addresses]
    tree2 = TreePay(payments=sum_pay, radix=2)
    total_amount = sum(x for (x, _) in sum_pay)
    example2 = UndoSend2(to_contract=tree2, from_contract=tree1, amount=total_amount, timeout=Days(10))

    CompilerWebSocket.set_example(example2)
    print(CompilerWebSocket.example_message)

    app = make_app()
    app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
