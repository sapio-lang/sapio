from __future__ import annotations

from typing import TypeVar, Generic, Any, Union, Optional

from typing_extensions import Protocol

from sapio.bitcoinlib.static_types import *
from sapio.util import methdispatch


class ClauseProtocol(Protocol):
    @property
    def a(self) -> Any:
        pass
    @property
    def b(self) -> Any:
        pass
    @property
    def n_args(self) -> int:
        return 0
    @property
    def symbol(self) -> str:
        return ""

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
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 0
class UnsatisfiableClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 0


class AndClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 2
    symbol = "*"

    def __init__(self, a: AndClauseArgument, b: AndClauseArgument):
        self.a = a
        self.b = b


class OrClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 2
    symbol = "+"
    def __init__(self, a: AndClauseArgument, b: AndClauseArgument):
        self.a: AndClauseArgument = a
        self.b: AndClauseArgument = b


class SignatureCheckClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 1
    def __init__(self, a: Variable[PubKey]):
        self.a = a
        self.b = a.sub_variable("signature")


class PreImageCheckClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 1

    a : Variable[Hash]
    b : Variable[Hash]
    def __init__(self, a: Variable[Hash]):
        self.a = a
        self.b = a.sub_variable("preimage")


class CheckTemplateVerifyClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 1

    def __init__(self, a: Variable[Hash]):
        self.a = a


import time

from datetime import datetime
class AbsoluteTimeSpec:
    MIN_DATE = 500_000_000

    def __init__(self, t):
        self.time : LockTime = t
    @staticmethod
    def from_date(d: datetime):
        secs = LockTime(uint32(d.timestamp()))
        if secs < AbsoluteTimeSpec.MIN_DATE:
            raise ValueError('Date In Past', min_date)
        return AbsoluteTimeSpec(secs)
    @staticmethod
    def at_height(d: int):
        if d > AbsoluteTimeSpec.MIN_DATE:
            raise ValueError("Too Many Blocks ", d, ">", AbsoluteTimeSpec.MIN_DATE)
        return AbsoluteTimeSpec(d)

    @staticmethod
    def WeeksFromTime(t1:datetime, t2:float):
        return AbsoluteTimeSpec(AbsoluteTimeSpec.from_date(t1).time + LockTime(uint32(t2*7*24*60*60)))
    @staticmethod
    def DaysFromTime(t1: datetime, t2: float):
        return AbsoluteTimeSpec(AbsoluteTimeSpec.from_date(t1).time + LockTime(uint32(t2*24*60*60)))
    @staticmethod
    def MonthsFromTime(t1: datetime, t2: float):
        return AbsoluteTimeSpec(AbsoluteTimeSpec.from_date(t1).time + LockTime(uint32(t2*30*24*60*60)))
    def __repr__(self):
        if self.time < AbsoluteTimeSpec.MIN_DATE:
            return "{}.at_height({})".format(self.__class__.__name__, self.time)
        else:
            return "{}({})".format(self.__class__.__name__, self.time)


class RelativeTimeSpec:
    def __init__(self, t):
        self.time : Sequence = t
    @staticmethod
    def from_seconds(seconds: float) -> RelativeTimeSpec:
        t = uint32((seconds + 511) // 512)
        if t > 0x0000ffff:
            raise ValueError("Time Span {} seconds is too long! ".format(seconds))
        # Bit 22 enables time based locks.
        l = uint32(1 << 22) | t
        return RelativeTimeSpec(Sequence(uint32(l)))


TimeSpec = Union[AbsoluteTimeSpec, RelativeTimeSpec]

def Weeks(n:float) -> RelativeTimeSpec:
    if n > (0xFFFF*512//60//60//24//7):
        raise ValueError("{} Week Span is too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n*7*24*60*60
    return RelativeTimeSpec.from_seconds(seconds)

def Days(n:float) -> RelativeTimeSpec:
    if n > (0xFFFF*512//60//60//24):
        raise ValueError("{} Day Span too long! ".format(n))
    # lock times are in groups of 512 seconds
    seconds = n*24*60*60
    return RelativeTimeSpec.from_seconds(seconds)


class AfterClause(StringClauseMixin):
    def __add__(self, other: AndClauseArgument) -> OrClause:
        return OrClause(self, other)

    def __mul__(self, other: AndClauseArgument) -> AndClause:
        return AndClause(self, other)
    n_args = 1

    @methdispatch
    def initialize(self, a: Variable[TimeSpec]):
        self.a = a
    @initialize.register
    def _with_relative(self, a: RelativeTimeSpec):
        self.a = Variable("", a)
    @initialize.register
    def _with_absolute(self, a: AbsoluteTimeSpec):
        self.a = Variable("", a)
    def __init__(self, a: Union[Variable[TimeSpec], TimeSpec]):
        self.initialize(a)



V = TypeVar('V')


class Variable(Generic[V]):
    def __init__(self, name: Union[bytes, str], value: Optional[V] = None):
        self.name: bytes = bytes(name, 'utf-8') if isinstance(name, str) else name
        self.assigned_value: Optional[V] = value
        self.sub_variable_count = -1

    def sub_variable(self, purpose: str, value: Optional[V] = None) -> Variable:
        self.sub_variable_count += 1
        return Variable(self.name + b"_" + bytes(str(self.sub_variable_count), 'utf-8') + b"_" + bytes(purpose, 'utf-8'), value)

    def assign(self, value: V):
        self.assigned_value = value

    def __str__(self):
        return "{}('{}', {})".format(self.__class__.__name__, self.name, self.assigned_value)


AndClauseArgument = Union[
               SatisfiedClause,
               UnsatisfiableClause,
               OrClause,
               AndClause,
               SignatureCheckClause,
               PreImageCheckClause,
               CheckTemplateVerifyClause,
               AfterClause]
Clause = Union[SatisfiedClause, UnsatisfiableClause,
               Variable,
               OrClause,
               AndClause,
               SignatureCheckClause,
               PreImageCheckClause,
               CheckTemplateVerifyClause,
               AfterClause]


