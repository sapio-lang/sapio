import unittest
from datetime import datetime

from sapio_bitcoinlib import segwit_addr
from sapio_bitcoinlib.address import script_to_p2wsh
from sapio_bitcoinlib.script import CScript
from sapio_bitcoinlib.static_types import Sats, Bitcoin
from sapio_zoo.p2pk import *
from sapio_zoo.subscription import *
from bitcoin_script_compiler.clause import Weeks
from sapio_bitcoinlib.messages import COutPoint


class MyTestCase(unittest.TestCase):
    def test_something(self):
        # amount: Amount
        # recipient: PayToSegwitAddress
        # schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
        # return_address: PayToSegwitAddress
        # watchtower_key: PubKey
        # return_timeout: RelativeTimeSpec
        alice_script = script_to_p2wsh(CScript([b"Alice's Key Goes Here!"]))
        bob_script = script_to_p2wsh(CScript([b"Bob's Key Goes Here!"]))
        Alice = PayToSegwitAddress(amount=Bitcoin(5), address=alice_script)
        Bob = PayToSegwitAddress(amount=Bitcoin(5), address=bob_script)
        watchtower_key = b"......"
        now = datetime.now()
        c = CancellableSubscription(
            amount=Bitcoin(5),
            recipient=Bob,
            schedule=[
                (AbsoluteTimeSpec.DaysFromTime(now, 5), Bitcoin(0.5)),
                (AbsoluteTimeSpec.WeeksFromTime(now, 4), Bitcoin(3)),
                (AbsoluteTimeSpec.MonthsFromTime(now, 5), Bitcoin(1.5)),
            ],
            return_address=Alice,
            watchtower_key=watchtower_key,
            return_timeout=Weeks(1),
        )
        c.bind(COutPoint())


if __name__ == "__main__":
    unittest.main()
