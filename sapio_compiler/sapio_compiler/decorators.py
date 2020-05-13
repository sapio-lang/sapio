"""
decorators.py
---------------------------

Decorators are class language functions which can be used to drive compilation
of contracts. This module contains both "convenience exports" of the decorators
and the functional modules that handle the combinators of the decorators.

"""

from __future__ import annotations

from functools import reduce
from itertools import combinations
from typing import Callable, Generic, Iterator, List, Optional, Tuple, TypeVar, Union

import sapio_compiler
from bitcoin_script_compiler.clause import Clause, SatisfiedClause
from bitcoinlib.static_types import Amount

from .core.txtemplate import TransactionTemplate

T = TypeVar("T")
T2 = TypeVar("T2")

ContractType = TypeVar("ContractType")

PathReturnType = Union[TransactionTemplate, Iterator[TransactionTemplate]]
PathFunctionType = Callable[[ContractType], PathReturnType]


class PathFunction(Generic[ContractType]):
    """
    A path function is a type of function which must return either a
    TransactionTemplate or an Iterator[TransactionTemplate].

    There are two fundamental ways of constructing a PathFunction, either for a
    guarantee-d path (using PathFunction.guarantee) or a unlock_but_suggeste-d
    path (using PathFunction.unlock_but_suggest). The difference is that
    guarantee instructs the compiler to use CheckTemplateVerify to ensure the
    outcomes whereas unlock_but_suggest does not.

    """
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(
        self,
        f: PathFunctionType[ContractType],
        unlocker: Callable[[ContractType], Clause],
        is_guaranteed,
    ) -> None:
        self.f: PathFunctionType[ContractType] = f
        self.unlock_with: Callable[[ContractType], Clause] = unlocker
        self.is_guaranteed: bool = is_guaranteed
        self.__name__ = f.__name__

    def __call__(self, obj: ContractType) -> PathReturnType:
        return self.f(obj)

    @staticmethod
    def guarantee(arg: PathFunctionType[ContractType]) -> PathFunction[ContractType]:
        """
        Create a guaranteed spending path, using CheckTemplateVerify
        """
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause(), True)

    @staticmethod
    def unlock_but_suggest(
        arg: PathFunctionType[ContractType],
    ) -> PathFunction[ContractType]:
        """
        Create a unlocked spending path, but return what the next step should
        be.

        This is useful for HTLC based protocols.
        """
        return PathFunction[ContractType](arg, lambda x: SatisfiedClause(), False)


class UnlockFunction(Generic[ContractType]):
    """
    An UnlockFunction expresses a keypath spending. There are no further
    restrictions on how a coin may be spent.
    """
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[ContractType], Clause], name: str) -> None:
        self.unlock_with = condition
        self.__name__ = name

    def __call__(self, obj: ContractType) -> Clause:
        return self.unlock_with(obj)

    @staticmethod
    def unlock(s: Callable[[ContractType], Clause]) -> UnlockFunction[ContractType]:
        """
        Create a unlocked spending path.
        """
        return UnlockFunction[ContractType](s, s.__name__)


class PayAddress(Generic[ContractType]):
    """
    A PayAddress function is a special type which stubs out
    the contract as being just the amount/address combo returned

    If a PayAddress decorator is used, no other functions may be present,
    except assertions.

    This is useful to avoid creating an intermediate txn to patch in an address.

    Mostly for use by Library Writers, but useful to expose in case one
    wants to add validation logic for passed in addresses (e.g., consulting
    a local node/wallet to check if the key is known).
    """
    def __init__(self, address: Callable[[ContractType], Tuple[Amount, str]]) -> None:
        self.address: Callable[[ContractType], Tuple[Amount, str]] = address

    def __call__(self, obj: ContractType) -> Tuple[Amount, str]:
        return self.address(obj)

    @staticmethod
    def pay_address(
        f: Callable[[ContractType], Tuple[Amount, str]]
    ) -> PayAddress[ContractType]:
        """Create a pay_address function"""
        return PayAddress(f)


class CheckFunction(Generic[ContractType]):
    """
    A CheckFunction decorator should return a function that either raises its
    own exception or returns True/False.

    Raising your own exception is preferable because it can help users
    debug their own contracts more readily.
    """
    def __init__(self, func: Callable[[ContractType], bool]) -> None:
        self.func: Callable[[ContractType], bool] = func
        self.__name__ = func.__name__

    def __call__(self, obj: ContractType) -> bool:
        return self.func(obj)

    @staticmethod
    def check(s: Callable[[ContractType], bool]) -> CheckFunction[ContractType]:
        """create a check"""
        return CheckFunction(s)


class LayeredRequirement(Generic[ContractType]):
    """
    Layered requirement allows one to create custom requirement decorators
    which can wrap UnlockFunctions or PathFunctions.

    This allows one to build up the set of conditions by which a particular
    branch may be spent.

    Examples
    --------
    >>> class A(Contract):
    ...     class Fields:
    ...         pk: PubKey
    ...     @require
    ...     def signed(self):
    ...         return SignatureCheckClause(self.pk)
    ...     @signed
    ...     @unlock
    ...     def spend(self):
    ...         return SatisfiedClause()

    note that if mypy complains you may modify the `@signed` decorator
    to `@signed.stack` if you wish to wrap a require with more conditions.
    """
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

            p: PathFunction[ContractType] = PathFunction[ContractType](
                pf.f, wrap_path, pf.is_guaranteed
            )
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
        """
        stack is required for mypy only, as the type of __call__ cannot be
        properly deduced.
        """
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
        """
        require declares a requirement variable, but does not
        enforce it.
        """
        return LayeredRequirement[ContractType](arg)

    @staticmethod
    def threshold(
        n: int, l: List[LayeredRequirement[ContractType]]
    ) -> LayeredRequirement[ContractType]:
        """
        """
        if not len(l) >= n:
            raise ValueError("Expected to get more conditions in threshold")
        if not n > 0:
            raise ValueError("Threshold int must be positive")

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


R = TypeVar("R")


#TODO: remove enable_if
def enable_if(b: bool) -> Union[Callable[[R], R], Callable[[R], None]]:
    """
    enable_if is useful to decorate classes that we only want to have a feature
    on for some known boolean conditional.  It should be used with a class
    factory.

    Note that generally, enable_if is not required as it's fine to just use
    and if/else statement instead.

    enable_if is deprecated
    """
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
