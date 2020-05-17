from typing import (
    Dict,
    Generic,
    List,
    Literal,
    Optional,
    Protocol,
    Tuple,
    Type,
    TypeVar,
    Union,
)

from bitcoin_script_compiler import (
    Clause,
    RevealPreImage,
    RelativeTimeSpec,
    Satisfied,
    SignedBy,
)
from sapio_bitcoinlib.static_types import Amount, Hash, PubKey
from sapio_compiler import (
    BindableContract,
    Contract,
    TransactionTemplate,
    enable_if,
    guarantee,
    require,
    unlock,
    unlock_but_suggest,
)
from sapio_zoo.p2pk import PayToPubKey, PayToSegwitAddress


class OPENING:
    pass


class CLOSING:
    pass


T = Union[Type[OPENING], Type[CLOSING]]

# memoize means only one instance of the type of class gets created
memoize: Dict[T, Type[BindableContract]] = {}

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
            return SignedBy(self.alice) & SignedBy(self.bob)

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
                next_tx.add_output(self.amount, self.initial)
            else:
                for (amt, addr) in state:
                    next_tx.add_output(
                        amt, PayToSegwitAddress(amount=amt, address=addr)
                    )
            next_tx.set_sequence(self.timeout)
            tx = TransactionTemplate()

            contest = ContestedChannelAfterUpdate(
                amount=self.amount,
                state=next_tx,
                revocation=revocation,
                honest=self.alice if proposer_id == "alice" else self.bob,
            )
            print(contest.amount_range)
            tx.add_output(self.amount.contest)
            return tx

        @cooperate
        @unlock
        def coop_close(self) -> Clause:
            return Satisfied()

        @enable_if(stage is OPENING)
        @guarantee
        def begin_contest(self) -> TransactionTemplate:
            tx = TransactionTemplate()
            tx.add_output(
                self.amount,
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
            tx.set_sequence(self.timeout)
            tx.add_output(self.amount, self.initial)
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
        t: TransactionTemplate = self.state
        return t

    @require
    def cheating_caught(self) -> Clause:
        return RevealPreImage(self.revocation) & SignedBy(self.honest)

    @cheating_caught
    @unlock_but_suggest
    def close_channel(
        self, tx_override: Optional[TransactionTemplate] = None
    ) -> TransactionTemplate:
        if tx_override is not None:
            return tx_override
        tx = TransactionTemplate()
        tx.add_output(self.amount, PayToPubKey(key=self.honest, amount=self.amount))
        return tx
