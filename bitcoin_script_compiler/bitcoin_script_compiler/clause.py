from __future__ import annotations

from datetime import datetime
from functools import singledispatchmethod
from typing import Any, List, Protocol, Type, Union, cast

from bitcoinlib.static_types import Hash, LockTime, PubKey, Sequence, uint32

from .variable import AssignedVariable, UnassignedVariable


class ClauseProtocol(Protocol):
    @property
    def a(self) -> Any:
        pass

    @property
    def b(self) -> Any:
        pass

    n_args: int
    symbol: str


class StringClauseMixin:
    MODE = "+"  # "str"

    def __str__(self: ClauseProtocol) -> str:
        if StringClauseMixin.MODE == "+":
            if self.__class__.n_args == 1:
                return "{}({})".format(self.__class__.__name__, self.a)
            elif self.__class__.n_args == 2:
                return "{}{}{}".format(self.a, self.symbol, self.b)
            else:
                return "{}()".format(self.__class__.__name__)
        else:
            if self.__class__.n_args == 1:
                return "{}({})".format(self.__class__.__name__, self.a)
            elif self.__class__.n_args == 2:
                return "{}({}, {})".format(self.__class__.__name__, self.a, self.b)
            else:
                return "{}()".format(self.__class__.__name__)


class SatisfiedClause(StringClauseMixin):
    # When or'd to another clause, the other clause disappears
    # because A + True --> True
    def __or__(self, other: Clause) -> SatisfiedClause:
        return self

    # When and'd to another clause, this clause disappears
    # because A*True --> A
    def __and__(self, other: Clause) -> Clause:
        return other

    n_args = 0


class UnsatisfiableClause(StringClauseMixin):
    # When or'd to another clause, this clause disappears
    # because A + False --> A

    # N.B.: This makes UnsatisfiableClause useful as a "None" value
    # for binary clauses
    def __or__(self, other: Clause) -> Clause:
        return other

    # When and'd to another clause, the other clause disappears
    # because A*False --> False
    def __and__(self, other: Clause) -> UnsatisfiableClause:
        return self

    n_args = 0


class LogicMixin:
    def __or__(self, other: Clause) -> OrClause:
        return OrClause(cast(Clause, self), other)

    def __and__(self, other: Clause) -> AndClause:
        return AndClause(cast(Clause, self), other)


class AndClause(LogicMixin, StringClauseMixin):
    n_args = 2
    symbol = "*"

    def __init__(self, a: Clause, b: Clause):
        self.a = a
        self.b = b


class OrClause(LogicMixin, StringClauseMixin):
    n_args = 2
    symbol = "+"

    def __init__(self, a: Clause, b: Clause):
        self.a: Clause = a
        self.b: Clause = b


class SignatureCheckClause(LogicMixin, StringClauseMixin):
    n_args = 1

    def __init__(self, a: AssignedVariable[PubKey]):
        self.a = a


class PreImageCheckClause(LogicMixin, StringClauseMixin):
    n_args = 1
    a: AssignedVariable[Hash]
    b: AssignedVariable[Hash]

    def __init__(self, a: AssignedVariable[Hash]):
        self.a = a


class CheckTemplateVerifyClause(LogicMixin, StringClauseMixin):
    n_args = 1

    def __init__(self, a: AssignedVariable[Hash]):
        self.a = a


class AbsoluteTimeSpec:
    class Blocks:
        pass

    class Time:
        pass

    Types = Union[Type[Blocks], Type[Time]]

    def get_type(self) -> AbsoluteTimeSpec.Types:
        return self.Blocks if self.time < self.MIN_DATE else self.Time

    MIN_DATE = 500_000_000

    def __init__(self, t: LockTime):
        self.time: LockTime = t

    @staticmethod
    def from_date(d: datetime) -> AbsoluteTimeSpec:
        secs = LockTime(uint32(d.timestamp()))
        if secs < AbsoluteTimeSpec.MIN_DATE:
            raise ValueError("Date In Past", AbsoluteTimeSpec.MIN_DATE)
        return AbsoluteTimeSpec(secs)

    @staticmethod
    def at_height(d: int) -> AbsoluteTimeSpec:
        if d > AbsoluteTimeSpec.MIN_DATE:
            raise ValueError("Too Many Blocks ", d, ">", AbsoluteTimeSpec.MIN_DATE)
        return AbsoluteTimeSpec(LockTime(d))

    @staticmethod
    def WeeksFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        base = AbsoluteTimeSpec.from_date(t1).time
        delta = LockTime(uint32(t2 * 7 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    @staticmethod
    def DaysFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        base = AbsoluteTimeSpec.from_date(t1).time
        delta = LockTime(uint32(t2 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    @staticmethod
    def MonthsFromTime(t1: datetime, t2: float) -> AbsoluteTimeSpec:
        base = AbsoluteTimeSpec.from_date(t1).time
        delta = LockTime(uint32(t2 * 30 * 24 * 60 * 60))
        return AbsoluteTimeSpec(LockTime(base + delta))

    def __repr__(self) -> str:
        if self.time < AbsoluteTimeSpec.MIN_DATE:
            return "{}.at_height({})".format(self.__class__.__name__, self.time)
        else:
            return "{}({})".format(self.__class__.__name__, self.time)


class RelativeTimeSpec:
    class Blocks:
        pass

    class Time:
        pass

    Types = Union[Type[Blocks], Type[Time]]

    def __init__(self, t: Sequence):
        self.time: Sequence = t

    @staticmethod
    def from_seconds(seconds: float) -> RelativeTimeSpec:
        t = uint32((seconds + 511) // 512)
        if t > 0x0000FFFF:
            raise ValueError("Time Span {} seconds is too long! ".format(seconds))
        # Bit 22 enables time based locks.
        l = uint32(1 << 22) | t
        return RelativeTimeSpec(Sequence(uint32(l)))

    def get_type(self) -> RelativeTimeSpec.Types:
        return self.Time if (self.time & uint32(1 << 22)) else self.Blocks


TimeSpec = Union[AbsoluteTimeSpec, RelativeTimeSpec]


def Weeks(n: float) -> RelativeTimeSpec:
    if n > (0xFFFF * 512 // 60 // 60 // 24 // 7):
        raise ValueError("{} Week Span is too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n * 7 * 24 * 60 * 60
    return RelativeTimeSpec.from_seconds(seconds)


def Days(n: float) -> RelativeTimeSpec:
    if n > (0xFFFF * 512 // 60 // 60 // 24):
        raise ValueError("{} Day Span too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n * 24 * 60 * 60
    return RelativeTimeSpec.from_seconds(seconds)


class AfterClause(LogicMixin, StringClauseMixin):
    n_args = 1
    a: AssignedVariable[TimeSpec]

    @singledispatchmethod
    def initialize(self, a: Any) -> None:
        raise ValueError("Unsupported Type")

    @initialize.register
    def _with_assigned(self, a: AssignedVariable[Union[RelativeTimeSpec, AbsoluteTimeSpec]]) -> None:
        # TODO: Remove when mypy updates...
        assert callable(self.initialize)
        self.initialize(a.assigned_value)

    @initialize.register
    def _with_relative(self, a: RelativeTimeSpec) -> None:
        self.a = AssignedVariable(a, "")

    @initialize.register
    def _with_absolute(self, a: AbsoluteTimeSpec) -> None:
        self.a = AssignedVariable(a, "")

    def __init__(self, a: Union[AssignedVariable[TimeSpec], TimeSpec]):
        # TODO: Remove when mypy updates...
        assert callable(self.initialize)
        self.initialize(a)


DNFClause = Union[
    SatisfiedClause,
    UnsatisfiableClause,
    SignatureCheckClause,
    PreImageCheckClause,
    CheckTemplateVerifyClause,
    AfterClause,
]

DNF = List[List[DNFClause]]

Clause = Union[OrClause, AndClause, DNFClause]
