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
)
from sapio_zoo.p2pk import PayToPubKey, PayToSegwitAddress
from dataclasses import dataclass


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

    @dataclass
    class MetaData:
        label: str = f"channel[{stage.__name__}]"
        color: str = "red"

    @dataclass
    class Fields:
        initial: Contract
        alice: PubKey
        bob: PubKey
        timeout: RelativeTimeSpec
        amount: Amount
        metadata: MetaData
    Self = Contract(f"ChannelState_{stage.__name__}", Fields, [])

    @Self.let
    def cooperate(self) -> Clause:
        return SignedBy(self.alice) & SignedBy(self.bob)

    @cooperate
    @Self.finish_or
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
    @Self.finish
    def coop_close(self) -> Clause:
        return Satisfied()

    if stage is OPENING:
        @Self.then
        def begin_contest(self) -> TransactionTemplate:
            tx = TransactionTemplate()
            closing = ChannelClassFactory(CLOSING)
            tx.add_output(
                self.amount,
                closing(closing.Props(
                    amount=self.amount,
                    initial=self.initial,
                    timeout=self.timeout,
                    alice=self.alice,
                    bob=self.bob,
                )),
            )
            return tx

    if stage is CLOSING:
        @Self.then
        def finish_contest(self) -> TransactionTemplate:
            tx = TransactionTemplate()
            tx.set_sequence(self.timeout)
            tx.add_output(self.amount, self.initial)
            return tx

    memoize[stage] = Self
    return Self


BasicContestedChannel = ChannelClassFactory(CLOSING)
BasicChannel = ChannelClassFactory(OPENING)


@dataclass
class MetaData:
    label: str = "revoke"
    color: str = "yellow"


@dataclass
class Props:
    amount: Amount
    state: TransactionTemplate
    revocation: Hash
    honest: PubKey
    metadata: MetaData
ContestedChannelAfterUpdate = Contract("ContestedChannelAfterUpdate", Props, [])


@ContestedChannelAfterUpdate.then
def close(self) -> TransactionTemplate:
    t: TransactionTemplate = self.state
    return t

@ContestedChannelAfterUpdate.let
def cheating_caught(self) -> Clause:
    return RevealPreImage(self.revocation) & SignedBy(self.honest)

@cheating_caught
@ContestedChannelAfterUpdate.finish_or
def close_channel(
    self, tx_override: Optional[TransactionTemplate] = None
) -> TransactionTemplate:
    if tx_override is not None:
        return tx_override
    tx = TransactionTemplate()
    tx.add_output(self.amount, PayToPubKey(key=self.honest, amount=self.amount))
    return tx
