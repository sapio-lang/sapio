from __future__ import annotations
from typing import TypeVar, Any, Union, Callable, List, Tuple, Iterable, Generic, Optional
from .txtemplate import TransactionTemplate
from sapio.script.clause import Clause, SatisfiedClause

from sapio.bitcoinlib.static_types import Amount

T = TypeVar("T")
T2 = TypeVar("T2")


PathReturnType = Union[
    TransactionTemplate, List[TransactionTemplate], Iterable[TransactionTemplate]
]
PathFunctionType = Callable[[T], PathReturnType]


class PathFunction(Generic[T]):
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, f: PathFunctionType[T], unlocker: Callable[[T], Clause]) -> None:
        self.f = f
        self.unlock_with = unlocker
        self.__name__ = f.__name__

    def __call__(self, obj:T) -> PathReturnType:
        return self.f(obj)


def path( arg: PathFunctionType[T]) -> PathFunction[T]:
    return PathFunction(arg, lambda _: SatisfiedClause())

def path_if( arg: Optional[Callable[[T], Clause]] = None) -> Callable[[PathFunctionType[T]], PathFunction[T]]:
    argw : Callable[[T], Clause] = (lambda _ : SatisfiedClause()) if arg is None else arg
    def wrapper(f: PathFunctionType[T]) -> PathFunction[T]:
        return PathFunction(f, lambda _: SatisfiedClause())
    return wrapper


class UnlockFunction:
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[T], Clause], name: str) -> None:
        self.unlock_with = condition
        self.__name__ = name

    def __call__(self, obj: T) -> Clause:
        return self.unlock_with(obj)


def unlock(
    s: Callable[[Any], Clause]
) -> Callable[[Callable[[T], None]], UnlockFunction]:
    def wrapper(f: Callable[[T], None]) -> UnlockFunction:
        return UnlockFunction(s, f.__name__)

    return wrapper


class PayAddress:

    def __init__(self, address: Callable[[T], Tuple[Amount, str]]) -> None:
        self.address: Callable[[T], Tuple[Amount, str]] = address

    def __call__(self, obj: T) -> Tuple[Amount, str]:
        return self.address(obj)


def pay_address(f: Callable[[T], Tuple[Amount, str]]) -> PayAddress:
    return PayAddress(f)


class CheckFunction:
    def __init__(self, func: Callable[[T], bool]) -> None:
        self.func : Callable[[T],bool] = func
        self.__name__ = func.__name__

    def __call__(self, obj:T)->bool:
        return self.func(obj)


def check(s: Callable[[T], bool]) -> CheckFunction:
    return CheckFunction(s)
