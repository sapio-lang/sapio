from typing import Callable, List, Tuple, Iterator

from sapio.examples.p2pk import PayToSegwitAddress
from sapio.examples.undo_send import UndoSend
from sapio.bitcoinlib.static_types import Amount, PubKey
from sapio.contract import Contract, TransactionTemplate, path
from sapio.spending_conditions.script_lang import TimeSpec, AbsoluteTimeSpec, RelativeTimeSpec, SignatureCheckClause


def add_timeout(tx, delay):
    if isinstance(delay, RelativeTimeSpec):
        tx.set_sequence(delay.time)
    elif isinstance(delay, AbsoluteTimeSpec):
        tx.set_locktime(delay.time)



class CancellableSubscription(Contract):
    class Fields:
        amount: Amount
        recipient: PayToSegwitAddress
        schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
        return_address: PayToSegwitAddress
        watchtower_key: PubKey
        return_timeout: RelativeTimeSpec

    @path
    def cancel(self):
        tx = TransactionTemplate()
        amount = self.amount.assigned_value
        return_address = self.return_address
        cc = CancelContest(
            amount=amount,
            recipient=self.recipient,
            schedule=self.schedule,
            return_address=self.return_address,
            watchtower_key=self.watchtower_key,
            return_timeout=self.return_timeout)
        tx.add_output(amount, cc)
        return tx

    @path
    def claim(self):
        tx = TransactionTemplate()
        (delay, amount) = self.schedule.assigned_value[0]
        add_timeout(tx, delay)

        total_amount = self.amount.assigned_value
        tx.add_output(amount, self.recipient.assigned_value)

        if len(self.schedule.assigned_value) > 1:
            new_amount = total_amount - amount
            tx.add_output(new_amount,
                          CancellableSubscription(amount=new_amount,
                                                  recipient=self.recipient,
                                                  schedule=self.schedule.assigned_value[1:],
                                                  watchtower_key=self.watchtower_key,
                                                  return_timeout=self.return_timeout,
                                                  return_address=self.return_address))
        return tx

class CancelContest(Contract):
    class Fields:
        amount: Amount
        recipient: Contract
        schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
        return_address: Contract
        watchtower_key: PubKey
        return_timeout: RelativeTimeSpec
    @path(lambda self: SignatureCheckClause(self.watchtower_key))
    def counterclaim(self) -> Iterator[TransactionTemplate]:
        total_amount = 0
        for (timeout, amount) in self.schedule.assigned_value:
            total_amount += amount
            tx = TransactionTemplate()
            tx.add_output(total_amount, self.recipient.assigned_value)
            refund = self.amount.assigned_value-total_amount
            tx.add_output(refund, self.return_address.assigned_value)
            add_timeout(tx, timeout)
            yield tx
    @path
    def finish_cancel(self):
        tx = TransactionTemplate()
        tx.set_sequence(self.return_timeout.assigned_value.time)
        amount = self.amount.assigned_value
        return_address = self.return_address.assigned_value
        tx.add_output(amount, return_address)
        return tx


