from __future__ import annotations

from typing import (
    Any,
    Callable,
    Generic,
    Iterator,
    List,
    Optional,
    Tuple,
    TypeVar,
    Union,
)

import sapio
from sapio.bitcoinlib.static_types import Amount
from sapio.script.clause import Clause, SatisfiedClause

from .txtemplate import TransactionTemplate

T = TypeVar("T")
T2 = TypeVar("T2")

ContractType = TypeVar("ContractType")

PathReturnType = Union[TransactionTemplate, Iterator[TransactionTemplate]]
PathFunctionType = Callable[[ContractType], PathReturnType]


class PathFunction(Generic[ContractType]):
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(
        self,
        f: PathFunctionType[ContractType],
        unlocker: Callable[[ContractType], Clause],
    ) -> None:
        self.f: PathFunctionType[ContractType] = f
        self.unlock_with: Callable[[ContractType], Clause] = unlocker
        self.__name__ = f.__name__

    def __call__(self, obj: ContractType) -> PathReturnType:
        return self.f(obj)

    @staticmethod
    def guarantee(arg: PathFunctionType[ContractType]) -> PathFunction[ContractType]:
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause())


class UnlockFunction(Generic[ContractType]):
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[ContractType], Clause], name: str) -> None:
        self.unlock_with = condition
        self.__name__ = name

    def __call__(self, obj: ContractType) -> Clause:
        return self.unlock_with(obj)

    @staticmethod
    def unlock(s: Callable[[ContractType], Clause]) -> UnlockFunction[ContractType]:
        return UnlockFunction[ContractType](s, s.__name__)



class PayAddress(Generic[ContractType]):
    def __init__(self, address: Callable[[ContractType], Tuple[Amount, str]]) -> None:
        self.address: Callable[[ContractType], Tuple[Amount, str]] = address

    def __call__(self, obj: ContractType) -> Tuple[Amount, str]:
        return self.address(obj)

    @staticmethod
    def pay_address(
        f: Callable[[ContractType], Tuple[Amount, str]]
    ) -> PayAddress[ContractType]:
        return PayAddress(f)


class CheckFunction(Generic[ContractType]):
    def __init__(self, func: Callable[[ContractType], bool]) -> None:
        self.func: Callable[[ContractType], bool] = func
        self.__name__ = func.__name__

    def __call__(self, obj: ContractType) -> bool:
        return self.func(obj)

    @staticmethod
    def check(s: Callable[[ContractType], bool]) -> CheckFunction[ContractType]:
        return CheckFunction(s)


class Wrapper(Generic[ContractType]):
    def __init__(self, arg: Callable[[ContractType], Clause]) -> None:
        self.arg: Callable[[ContractType], Clause] = arg

    def __call__(self, pf: PathFunction[ContractType]) -> PathFunction[ContractType]:
        p: PathFunction[ContractType] = PathFunction[ContractType](
            pf.f, lambda x: pf.unlock_with(x) & self.arg(x)
        )
        return p


class LayeredRequirement(Generic[ContractType]):
    @staticmethod
    def require(
        arg: Callable[[ContractType], Clause]
    ) -> Callable[[PathFunction[ContractType]], PathFunction[ContractType]]:
        return Wrapper[ContractType](arg)


guarantee = PathFunction.guarantee
require = LayeredRequirement.require
# TODO: Unify these two and make guarantee a modifier of the above?
unlock = UnlockFunction.unlock
pay_address = PayAddress.pay_address
check = CheckFunction.check
