from __future__ import annotations

from typing import (Any, Callable, Generic, Iterator, List, Optional, Tuple,
                    TypeVar, Union)

import sapio
from sapio.bitcoinlib.static_types import Amount
from sapio.script.clause import Clause, SatisfiedClause

from .txtemplate import TransactionTemplate

T = TypeVar("T")
T2 = TypeVar("T2")

ContractType = TypeVar("ContractType")

PathReturnType = Union[ TransactionTemplate, Iterator[TransactionTemplate] ]
PathFunctionType = Callable[[ContractType], PathReturnType]


class PathFunction(Generic[ContractType]):
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, f: PathFunctionType[ContractType], unlocker: Callable[[ContractType], Clause]) -> None:
        self.f = f
        self.unlock_with = unlocker
        self.__name__ = f.__name__

    def __call__(self, obj:ContractType) -> PathReturnType:
        return self.f(obj)
    @staticmethod
    def path_if(arg: Callable[[ContractType], Clause]) -> Callable[[PathFunctionType[ContractType]], PathFunction]:
        return lambda x: PathFunction[ContractType](x, arg)
    @staticmethod
    def path( arg: PathFunctionType[ContractType]) -> PathFunction[ContractType]:
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause())

path_if = PathFunction.path_if
path = PathFunction.path

class UnlockFunction(Generic[ContractType]):
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[ContractType], Clause], name: str) -> None:
        self.unlock_with = condition
        self.__name__ = name

    def __call__(self, obj: ContractType) -> Clause:
        return self.unlock_with(obj)


def unlock(
    s: Callable[[Any], Clause]
) -> Callable[[Callable[[T], None]], UnlockFunction]:
    def wrapper(f: Callable[[T], None]) -> UnlockFunction:
        return UnlockFunction(s, f.__name__)

    return wrapper


class PayAddress(Generic[ContractType]):

    def __init__(self, address: Callable[[ContractType], Tuple[Amount, str]]) -> None:
        self.address: Callable[[ContractType], Tuple[Amount, str]] = address

    def __call__(self, obj: ContractType) -> Tuple[Amount, str]:
        return self.address(obj)


def pay_address(f: Callable[[ContractType], Tuple[Amount, str]]) -> PayAddress:
    return PayAddress(f)


class CheckFunction(Generic[ContractType]):
    def __init__(self, func: Callable[[ContractType], bool]) -> None:
        self.func : Callable[[ContractType],bool] = func
        self.__name__ = func.__name__

    def __call__(self, obj:ContractType)->bool:
        return self.func(obj)


def check(s: Callable[[ContractType], bool]) -> CheckFunction:
    return CheckFunction(s)
