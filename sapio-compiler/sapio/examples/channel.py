from typing import Generic, List, Literal, Optional, Protocol, Tuple, Type, TypeVar, Union, Dict

from bitcoinlib.static_types import Amount, Hash, PubKey
from sapio.contract.contract import Contract
from sapio.contract.bindable_contract import BindableContract
from sapio.contract.decorators import guarantee, require, unlock, unlock_but_suggest, enable_if
from sapio.contract.txtemplate import TransactionTemplate
from sapio.examples.p2pk import PayToPubKey, PayToSegwitAddress
from sapio.script.clause import (
    PreImageCheckClause,
    RelativeTimeSpec,
    SatisfiedClause,
    SignatureCheckClause,
    Clause,
)
from sapio.script.variable import AssignedVariable


class OPENING: pass
class CLOSING: pass
T = Union[Type[OPENING], Type[CLOSING]]

# memoize means only one instance of the type of class gets created
memoize: Dict[T, Type[BindableContract]]= {}

# We use a class factory here because inehritence isn't really the right model
# for logically different contracts & Mixins don't work presently.
def ChannelClassFactory(stage: T):
    if stage in memoize:
        return memoize[stage]
    class Self(Contract):
        class Fields:
            initial: Contract
            alice: PubKey
            bob: PubKey
            timeout: RelativeTimeSpec
            amount: Amount

        class MetaData:
            label = lambda s: "BASE"
            color = lambda s: "blue"

        @require
        def cooperate(self) -> Clause:
            return SignatureCheckClause(self.alice) & SignatureCheckClause(self.bob)

        @cooperate
        @unlock_but_suggest
        def update_state(
            self,
            state: Optional[List[Tuple[Amount, str]]] = None,
            proposer_id: Literal["alice", "bob"] = "alice",
            revocation: Hash = Hash(b""),
        ) -> TransactionTemplate:
            next_tx = TransactionTemplate()
            if state is None:
                next_tx.add_output(self.amount.assigned_value, self.initial.assigned_value)
            else:
                for (amt, addr) in state:
                    next_tx.add_output(amt, PayToSegwitAddress(amount=amt, address=addr))
            next_tx.set_sequence(self.timeout.assigned_value.time)
            tx = TransactionTemplate()

            contest = ContestedChannelAfterUpdate(
                amount=self.amount,
                state=next_tx,
                revocation=revocation,
                honest=self.alice if proposer_id == "alice" else self.bob,
            )
            print(contest.amount_range)
            tx.add_output(self.amount.assigned_value, contest)
            return tx

        @cooperate
        @unlock
        def coop_close(self) -> Clause:
            return SatisfiedClause()
        @enable_if(stage is OPENING)
        @guarantee
        def begin_contest(self) -> TransactionTemplate:
            tx = TransactionTemplate()
            tx.add_output(
                self.amount.assigned_value,
                ChannelClassFactory(CLOSING)(
                    amount=self.amount,
                    initial=self.initial,
                    timeout=self.timeout,
                    alice=self.alice,
                    bob=self.bob,
                ),
            )
            return tx
        @enable_if(stage is CLOSING)
        @guarantee
        def finish_contest(self) -> TransactionTemplate:
            tx = TransactionTemplate()
            tx.set_sequence(self.timeout.assigned_value.time)
            tx.add_output(self.amount.assigned_value, self.initial.assigned_value)
            return tx
    memoize[stage] = Self
    return Self

BasicContestedChannel = ChannelClassFactory(CLOSING)
BasicChannel = ChannelClassFactory(OPENING)


class ContestedChannelAfterUpdate(Contract):
    class Fields:
        amount: Amount
        state: TransactionTemplate
        revocation: Hash
        honest: PubKey
    class MetaData:
        label = lambda s: "revoke"
        color = lambda s: "yellow"

    @guarantee
    def close(self) -> TransactionTemplate:
        t: TransactionTemplate = self.state.assigned_value
        return t

    @require
    def cheating_caught(self) -> Clause:
        return PreImageCheckClause(self.revocation) & SignatureCheckClause(self.honest)

    @cheating_caught
    @unlock_but_suggest
    def close_channel(
        self, tx_override: Optional[TransactionTemplate] = None
    ) -> TransactionTemplate:
        if tx_override is not None:
            return tx_override
        tx = TransactionTemplate()
        tx.add_output(
            self.amount.assigned_value, PayToPubKey(key=self.honest, amount=self.amount)
        )
        return tx
