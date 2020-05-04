
"""
This shows a token airdrop contract that periodically issues coins to a set of
participants on a given schedule...
"""
from typing import Tuple, List

from bitcoinlib.static_types import Amount, Sats
from sapio_compiler.contract import Contract, guarantee, TransactionTemplate
from sapio_zoo.tree_pay import TreePay
from bitcoin_script_compiler.clause import TimeSpec, RelativeTimeSpec, AbsoluteTimeSpec


class AirDrop(Contract):
    class Fields:
        batches : List[Tuple[TimeSpec, List[Tuple[Amount, Contract]]]]
        radix : int
    @guarantee
    def payout(self):
        tx = TransactionTemplate()
        delay, current_batch = self.batches.value[0]
        total_amt : Amount = sum([amt for (amt, to) in current_batch])
        tx.add_output(total_amt, TreePay(payments=current_batch, radix=self.radix))
        if isinstance(delay, RelativeTimeSpec):
            tx.set_sequence(delay.time)
        elif isinstance(delay, AbsoluteTimeSpec):
            tx.lock_time = delay.time
        if len(self.batches.value) > 1:
            remaining :Amount = Sats(0)
            for batch in self.batches.value[1:]:
                remaining += sum([amt for (amt, to) in current_batch])
            tx.add_output(remaining, AirDrop(batches = self.batches.value[1:], radix=self.radix))
        return tx
