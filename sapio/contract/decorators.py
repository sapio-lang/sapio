from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import (
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
        is_guaranteed=True,
    ) -> None:
        self.f: PathFunctionType[ContractType] = f
        self.unlock_with: Callable[[ContractType], Clause] = unlocker
        self.is_guaranteed: bool = is_guaranteed
        self.__name__ = f.__name__

    def __call__(self, obj: ContractType) -> PathReturnType:
        return self.f(obj)

    @staticmethod
    def guarantee(arg: PathFunctionType[ContractType]) -> PathFunction[ContractType]:
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause(), True)

    @staticmethod
    def unlock_but_suggest(arg: PathFunctionType[ContractType]) -> PathFunction[ContractType]:
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause(), False)


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


class LayeredRequirement(Generic[ContractType]):
    def __init__(self, arg: Callable[[ContractType], Clause]) -> None:
        self.arg: Callable[[ContractType], Clause] = arg

    def __call__(
        self,
        decorated: Union[
            UnlockFunction[ContractType],
            PathFunction[ContractType],
            LayeredRequirement[ContractType],
        ],
    ) -> Union[
        UnlockFunction[ContractType],
        PathFunction[ContractType],
        LayeredRequirement[ContractType],
    ]:
        if isinstance(decorated, UnlockFunction):
            uf: UnlockFunction[ContractType] = decorated

            def wrap_unlock(contract: ContractType) -> Clause:
                return uf(contract) & self.arg(contract)

            u: UnlockFunction[ContractType] = UnlockFunction[ContractType](
                wrap_unlock, uf.__name__
            )
            return u
        elif isinstance(decorated, PathFunction):
            pf: PathFunction[ContractType] = decorated

            def wrap_path(contract: ContractType) -> Clause:
                return pf.unlock_with(contract) & self.arg(contract)
                return pf.unlock_with(contract) & self.arg(contract)

            p: PathFunction[ContractType] = PathFunction[ContractType](pf.f, wrap_path)
            return p
        elif isinstance(decorated, LayeredRequirement):
            l: LayeredRequirement[ContractType] = decorated
            return self.stack(l)
        else:
            raise ValueError(
                "Applied to wrong type! Maybe you're missing a decorator in the stack..."
            )

    def stack(
        self, decorated: LayeredRequirement[ContractType]
    ) -> LayeredRequirement[ContractType]:
        def wrap_layer(contract: ContractType) -> Clause:
            return decorated.arg(contract) & self.arg(contract)

        f: LayeredRequirement[ContractType] = LayeredRequirement[ContractType](
            wrap_layer
        )
        return f

    @staticmethod
    def require(
        arg: Callable[[ContractType], Clause]
    ) -> LayeredRequirement[ContractType]:
        return LayeredRequirement[ContractType](arg)

    @staticmethod
    def threshold(
        n: int, l: List[LayeredRequirement[ContractType]]
    ) -> LayeredRequirement[ContractType]:
        assert len(l) >= n
        assert n > 0

        def wrapper(self: ContractType) -> Clause:
            conds = []
            for arg in l:
                # take inner requirement
                conds.append(arg.arg(self))
            l3 = [
                reduce(lambda a, b: a & b, combo[1:], combo[0])
                for combo in combinations(conds, n)
            ]
            return reduce(lambda a, b: a | b, l3[1:], l3[0])

        return LayeredRequirement[ContractType](wrapper)


# enable_if is useful to decorate classes that we only want to have a feature on
# for some known boolean conditional.
# It should be used with a class factory
R = TypeVar("R")
def enable_if(b: bool) -> Union[Callable[[R], R], Callable[[R], None]]:
    if b:
        return lambda f: f
    else:
        return lambda f: None

guarantee = PathFunction.guarantee
require = LayeredRequirement.require
# TODO: Unify these two and make guarantee a modifier of the above?
unlock = UnlockFunction.unlock
unlock_but_suggest = PathFunction.unlock_but_suggest
pay_address = PayAddress.pay_address
check = CheckFunction.check
threshold = LayeredRequirement.threshold
