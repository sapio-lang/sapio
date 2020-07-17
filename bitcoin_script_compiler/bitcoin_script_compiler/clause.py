"""
clause.py
===============================
Types of logical clauses allowed within a transaction.

Both conjunctive & base types defined here.


Every clause should represent an operation that can be represented as a script
operation that reads some data from the stack and self-verifies correctness.

Each operation should consume its arguments and not leave anything new on the
stack.

The actual logic for these clauses is contained in other files, these are just
data containers located here. This modular design is a bit easier to work with
as it helps keep similar logic passes local.
"""

from __future__ import annotations

from datetime import datetime
from functools import singledispatchmethod
from typing import Any, List, Protocol, Union, cast, Literal, TYPE_CHECKING

from sapio_bitcoinlib.static_types import Hash, LockTime, PubKey, Sequence, uint32
from sapio_bitcoinlib.key import ECPubKey


class ClauseProtocol(Protocol):
    @property
    def a(self) -> Any:
        pass

    @property
    def b(self) -> Any:
        pass


class StringClauseMixin:
    """Mixin to add str printing"""

    def __str__(self: ClauseProtocol) -> str:
        if StringClauseMixin.MODE == "+":
            if self.__class__.n_args == 1:
                return f"{self.__class__.__name__}({self.a})"
            elif self.__class__.n_args == 2:
                return f"{self.a}{self.symbol}{self.b}"
            else:
                return f"{self.__class__.__name__}()"


class LogicMixin:
    """Mixin to add logic syntax to a class"""

    def __or__(self, other: Clause) -> Or:
        return Or(cast(Clause, self), other)

    def __and__(self, other: Clause) -> And:
        return And(cast(Clause, self), other)


class Satisfied:
    """A Base type clause which is always true. Useful in compiler logic."""

    # When or'd to another clause, the other clause disappears
    # because A + True --> True
    def __or__(self, other: Clause) -> Satisfied:
        return self

    # When and'd to another clause, this clause disappears
    # because A*True --> A
    def __and__(self, other: Clause) -> Clause:
        return other

    def __eq__(self, other: Clause) -> bool:
        return isinstance(other, Satisfied)

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}()"

    def to_miniscript(self):
        return "1"


class Unsatisfiable:
    """A Base type clause which is always false. Useful in compiler logic."""

    # When or'd to another clause, this clause disappears
    # because A + False --> A

    # N.B.: This makes UnsatisfiableClause useful as a "None" value
    # for binary clauses
    def __or__(self, other: Clause) -> Clause:
        return other

    # When and'd to another clause, the other clause disappears
    # because A*False --> False
    def __and__(self, other: Clause) -> Unsatisfiable:
        return self

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}()"

    def to_miniscript(self):
        return "0"


class BinaryLogicClause:
    def __init__(self, a: Clause, b: Clause):
        self.left = a
        self.right = b

    def __repr__(self) -> str:
        return f"({self.left!r} {self.symbol} {self.right!r})"


class And(BinaryLogicClause, LogicMixin):
    """Expresses that both the left hand and right hand arguments must be satisfied."""

    symbol = "&"

    def to_miniscript(self):
        return f"and({self.left.to_miniscript()},{self.right.to_miniscript()})"


class Or(BinaryLogicClause, LogicMixin):
    """Expresses that either the left hand or right hand arguments must be satisfied."""

    symbol = "|"

    def to_miniscript(self):
        return f"or({self.left.to_miniscript()},{self.right.to_miniscript()})"


class SignedBy(LogicMixin):
    """Requires a signature from the passed in key to be satisfied"""

    def __init__(self, a: PubKey):
        self.pubkey = a

    def __repr__(self):
        return f"{self.__class__.__name__}({self.pubkey!r})"

    def to_miniscript(self):
        return f"pk({self.pubkey.get_bytes().hex()})"


class RevealPreImage(LogicMixin):
    """Requires a preimage of the passed in hash to be revealed to be satisfied"""

    preimage: Hash

    def __init__(self, a: Hash):
        self.image = a

    def __repr__(self):
        return f"{self.__class__.__name__}({self.preimage!r})"

    def to_miniscript(self):
        return f"sha256({self.image.hex()})"


class CheckTemplateVerify(LogicMixin):
    """The transaction must match the passed in StandardTemplateHash exactly for this clause to be satisfied"""

    def __init__(self, a: Hash):
        self.hash = a

    def __repr__(self):
        return f"{self.__class__.__name__}({self.hash!r})"

    def to_miniscript(self):
        return f"txtmpl({self.hash.hex()})"


TimeTypes = Union[Literal["time"], Literal["blocks"]]


class AbsoluteTimeSpec:
    """An nLockTime specification, either in Blocks or MTP Time"""

    def get_type(self) -> TimeTypes:
        """Return if the AbsoluteTimeSpec is a block or MTP time"""
        return "blocks" if self.locktime < self.MIN_DATE else "time"

    MIN_DATE = 500_000_000
    """The Threshold at which an int should be read as a date"""

    def __init__(self, t: LockTime):
        """Create a Spec from a LockTime"""
        self.locktime: LockTime = t

    @staticmethod
    def from_date(d: datetime) -> AbsoluteTimeSpec:
        """Create an AbsoluteTimeSpec from a given datetime object"""
        secs = LockTime(uint32(d.timestamp()))
        if secs < AbsoluteTimeSpec.MIN_DATE:
            raise ValueError("Date In Past", AbsoluteTimeSpec.MIN_DATE)
        return AbsoluteTimeSpec(secs)

    @staticmethod
    def at_height(d: int) -> AbsoluteTimeSpec:
        """Create an AbsoluteTimeSpec from a given height"""
        if d > AbsoluteTimeSpec.MIN_DATE:
            raise ValueError("Too Many Blocks ", d, ">", AbsoluteTimeSpec.MIN_DATE)
        return AbsoluteTimeSpec(LockTime(d))

    @staticmethod
    def WeeksFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        """Create an AbsoluteTimeSpec t2 weeks from the given datetime"""
        base = AbsoluteTimeSpec.from_date(t1).locktime
        delta = LockTime(uint32(t2 * 7 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    @staticmethod
    def DaysFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        """Create an AbsoluteTimeSpec t2 days from the given datetime"""
        base = AbsoluteTimeSpec.from_date(t1).locktime
        delta = LockTime(uint32(t2 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    @staticmethod
    def MonthsFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        """Create an AbsoluteTimeSpec t2 months from the given datetime"""
        base = AbsoluteTimeSpec.from_date(t1).locktime
        delta = LockTime(uint32(t2 * 30 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    def __repr__(self) -> str:
        if self.locktime < AbsoluteTimeSpec.MIN_DATE:
            return f"{self.__class__.__name__}.at_height({self.locktime})"
        else:
            return f"{self.__class__.__name__}({self.locktime})"

    def to_miniscript(self):
        return f"after({self.locktime})"


class RelativeTimeSpec:
    def __init__(self, t: Sequence):
        self.sequence: Sequence = t

    @staticmethod
    def from_seconds(seconds: float) -> RelativeTimeSpec:
        """Create a relative Timelock from the given number of seconds"""
        t = uint32((seconds + 511) // 512)
        if t > 0x0000FFFF:
            raise ValueError("Time Span {} seconds is too long! ".format(seconds))
        # Bit 22 enables time based locks.
        l = uint32(1 << 22) | t
        return RelativeTimeSpec(Sequence(uint32(l)))

    @staticmethod
    def blocks_later(t: int) -> RelativeTimeSpec:
        if t & 0x00FFFF != t:
            raise ValueError("Time Span {t} blocks is too large!")
        return RelativeTimeSpec(Sequence(t))

    def get_type(self) -> TimeTypes:
        return "time" if (self.sequence & uint32(1 << 22)) else "blocks"

    def __repr__(self) -> str:
        t = self.get_type()
        if t == "time":
            return f"{self.__class__.__name__}.from_seconds({self.locktime&0x00FFFF})"
        elif t == "blocks":
            return f"{self.__class__.__name__}.blocks_later({self.locktime & 0x00FFFF})"

    def to_miniscript(self):
        return f"older({self.sequence})"


TimeSpec = Union[AbsoluteTimeSpec, RelativeTimeSpec]


def Weeks(n: float) -> RelativeTimeSpec:
    """Create a relative Timelock from the given number of weeks"""
    if n > (0xFFFF * 512 // 60 // 60 // 24 // 7):
        raise ValueError("{} Week Span is too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n * 7 * 24 * 60 * 60
    return RelativeTimeSpec.from_seconds(seconds)


def Days(n: float) -> RelativeTimeSpec:
    """Create a relative Timelock from the given number of Days"""
    if n > (0xFFFF * 512 // 60 // 60 // 24):
        raise ValueError("{} Day Span too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n * 24 * 60 * 60
    return RelativeTimeSpec.from_seconds(seconds)


class Wait(LogicMixin):
    """Takes either a RelativeTimeSpec or an AbsoluteTimeSpec and enforces the condition"""

    time: TimeSpec

    @singledispatchmethod
    def initialize(self, a: Any) -> None:
        raise ValueError("Unsupported Type")

    @initialize.register
    def _with_relative(self, a: RelativeTimeSpec) -> None:
        self.time = a

    @initialize.register
    def _with_absolute(self, a: AbsoluteTimeSpec) -> None:
        self.time = a

    def __init__(self, a: TimeSpec):
        # TODO: Remove when mypy updates...
        if TYPE_CHECKING:
            assert callable(self.initialize)
        self.initialize(a)

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}({self.time!r})"

    def to_miniscript(self):
        return self.time.to_miniscript()


class Threshold(LogicMixin):
    """Takes a list of clauses and a threshold"""

    thresh: int

    clauses: List[DNFClause]

    def __init__(self, thresh: int, clauses: Union[List[Clause], List[ECPubKey]]):
        self.thresh = thresh
        self.clauses = clauses

    def to_miniscript(self) -> str:
        if all(isinstance(c, ECPubKey) for c in self.clauses):
            s = ",".join([f"pk({c.get_bytes().hex()})" for c in self.clauses])
            return f"thresh({self.thresh},{s})"
        else:
            # Wrap each clause so that it's dissatisfiable trivially
            # But also so that when satisified, it's a B type
            s = ",".join([cl.to_miniscript() for cl in self.clauses])
            return f"thresh({self.thresh},{s})"


DNFClause = Union[
    Satisfied,
    Unsatisfiable,
    SignedBy,
    RevealPreImage,
    CheckTemplateVerify,
    Wait,
    Threshold,
]
"""DNF Clauses are basic types of clauses that can't be reduced further."""


DNF = List[List[DNFClause]]
"""Every element in the base list is AND'd together, every list in the outer list is OR'd"""

Clause = Union[Or, And, DNFClause]
"""Clause includes AndClause and OrClause in addition to DNFClause"""
