"""
decorators.py
---------------------------

Decorators are class language functions which can be used to drive compilation
of contracts. This module contains both "convenience exports" of the decorators
and the functional modules that handle the combinators of the decorators.

"""

from __future__ import annotations

from functools import reduce, wraps
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
    Literal,
    Any,
    Type,
)

import sapio_compiler
from bitcoin_script_compiler.clause import Clause, Satisfied, Threshold
from sapio_bitcoinlib.static_types import Amount

from .core.txtemplate import TransactionTemplate
from .core.bindable_contract import AmountRange

import sapio_compiler.core.bindable_contract

ContractType = TypeVar(
    "ContractType", bound="sapio_compiler.core.bindable_contract.BindableContract[Any]"
)


from typing import Protocol, cast


# Protocols for Function Types
FuncTypes = Union[
    Literal["path"],
    Literal["unlock"],
    Literal["check"],
    Literal["pay_address"],
    Literal["require"],
]


PathReturnType = Union[TransactionTemplate, Iterator[TransactionTemplate]]


class PathFunction(Protocol[ContractType]):
    unlock_with: Optional[Callable[[ContractType], Clause]]
    is_guaranteed: bool
    __sapio_func_type__: FuncTypes = "path"

    @staticmethod
    def __call__(self: ContractType) -> PathReturnType:
        pass

    __name__: str


class PayFunction(Protocol[ContractType]):
    # this silences issues around the type needing to be contravariant
    __silence_error: Type[ContractType]
    __sapio_func_type__: FuncTypes = "pay_address"

    @staticmethod
    def __call__(self: ContractType) -> Tuple[AmountRange, str]:
        pass


class UnlockFunction(Protocol[ContractType]):
    # this silences issues around the type needing to be contravariant
    __silence_error: Type[ContractType]
    __sapio_func_type__: FuncTypes = "unlock"

    @staticmethod
    def __call__(self: ContractType) -> Clause:
        pass


class CheckFunction(Protocol[ContractType]):
    # this silences issues around the type needing to be contravariant
    __silence_error: Type[ContractType]
    __sapio_func_type__: FuncTypes = "check"

    @staticmethod
    def __call__(self: ContractType) -> bool:
        pass


class RequireFunction(Protocol[ContractType]):
    __sapio_func_type__: FuncTypes = "require"

    @staticmethod
    def __call__(
        self: Union[
            RequireFunction[ContractType],
            PathFunction[ContractType],
            UnlockFunction[ContractType],
        ]
    ) -> RequireFunction[ContractType]:
        pass

    original_func: Optional[Callable[[ContractType], Clause]]


RequireWrappable = Union[
    RequireFunction[ContractType],
    PathFunction[ContractType],
    UnlockFunction[ContractType],
]

AllFuncs = Union[
    RequireWrappable[ContractType],
    CheckFunction[ContractType],
    PayFunction[ContractType],
]

FUNC_TYPE_TAG = "__sapio_func_type__"


def tag(a: AllFuncs[ContractType], b: FuncTypes) -> None:
    if hasattr(a, FUNC_TYPE_TAG):
        raise AlreadyDecorated()
    setattr(a, FUNC_TYPE_TAG, b)


def get_type_tag(a: Any) -> Optional[FuncTypes]:
    if not hasattr(a, FUNC_TYPE_TAG):
        return None
    else:
        s = getattr(a, FUNC_TYPE_TAG)
        if s == "require":
            return "require"
        if s == "path":
            return "path"
        if s == "check":
            return "check"
        if s == "unlock":
            return "unlock"
        if s == "pay_address":
            return "pay_address"
        raise ValueError("Unknown type")


class AlreadyDecorated(Exception):
    pass


def satisfied(x: ContractType) -> Satisfied:
    return Satisfied()


def guarantee(
    arg: Callable[[ContractType], PathReturnType]
) -> PathFunction[ContractType]:
    """
    A path function is a type of function which must return either a
    TransactionTemplate or an Iterator[TransactionTemplate].

    There are two fundamental ways of constructing a PathFunction, either for a
    guarantee-d path (using PathFunction.guarantee) or a unlock_but_suggeste-d
    path (using PathFunction.unlock_but_suggest). The difference is that
    guarantee instructs the compiler to use CheckTemplateVerify to ensure the
    outcomes whereas unlock_but_suggest does not.
    """
    cast_arg = cast(PathFunction[ContractType], arg)
    tag(cast_arg, "path")
    cast_arg.unlock_with = satisfied
    cast_arg.is_guaranteed = True
    return cast_arg


def unlock_but_suggest(
    arg: Callable[[ContractType], PathReturnType]
) -> PathFunction[ContractType]:
    """
    A path function is a type of function which must return either a
    TransactionTemplate or an Iterator[TransactionTemplate].

    There are two fundamental ways of constructing a PathFunction, either for a
    guarantee-d path (using PathFunction.guarantee) or a unlock_but_suggeste-d
    path (using PathFunction.unlock_but_suggest). The difference is that
    guarantee instructs the compiler to use CheckTemplateVerify to ensure the
    outcomes whereas unlock_but_suggest does not.

    This is useful for HTLC based protocols.
    """
    cast_arg = cast(PathFunction[ContractType], arg)
    tag(cast_arg, "path")
    cast_arg.unlock_with = satisfied
    cast_arg.is_guaranteed = False
    return cast_arg


def unlock(s: Callable[[ContractType], Clause]) -> UnlockFunction[ContractType]:
    """
    An UnlockFunction expresses a keypath spending. There are no further
    restrictions on how a coin may be spent.
    """
    cast_s = cast(UnlockFunction[ContractType], s)
    tag(cast_s, "unlock")
    return cast_s


def pay_address(
    f: Callable[[ContractType], Tuple[AmountRange, str]]
) -> PayFunction[ContractType]:
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
    cast_f = cast(PayFunction[ContractType], f)
    tag(cast_f, "pay_address")
    return cast_f


def check(s: Callable[[ContractType], bool]) -> CheckFunction[ContractType]:
    """
    A CheckFunction decorator should return a function that either raises its
    own exception or returns True/False.

    Raising your own exception is preferable because it can help users
    debug their own contracts more readily.
    """
    cast_s = cast(CheckFunction[ContractType], s)
    tag(cast_s, "check")
    return cast_s


def require(arg: Callable[[ContractType], Clause]) -> RequireFunction[ContractType]:
    """
    require declares a requirement variable, but does not
    enforce it.

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
    ...         return Satisfied()
    """

    @wraps(arg)
    def inner(
        decorated: RequireWrappable[ContractType],
    ) -> RequireWrappable[ContractType]:
        if get_type_tag(decorated) == "unlock":
            uf: UnlockFunction[ContractType] = cast(
                UnlockFunction[ContractType], decorated
            )

            @wraps(decorated)
            def wrap_unlock(contract: ContractType) -> Clause:
                # TODO: Is this correct? mypy fails...
                d: Clause = uf(contract)
                cl: Clause = arg(contract)
                return d & cl

            c = cast(UnlockFunction[ContractType], wrap_unlock)
            assert get_type_tag(c) == "unlock"
            return c
        elif get_type_tag(decorated) == "path":
            pf: PathFunction[ContractType] = cast(PathFunction[ContractType], decorated)
            former = pf.unlock_with or satisfied

            def wrap_path(contract: ContractType) -> Clause:
                return former(contract) & arg(contract)

            pf.unlock_with = wrap_path
            return pf
        elif get_type_tag(decorated) == "require":
            # re-wrap this in require to keep the decorator chain going
            rf = cast(RequireFunction[ContractType], decorated)

            @require
            def wrap_layer(contract: ContractType) -> Clause:
                # We still know arg for the current req, but
                # decorated is now an opaque decorator.
                # So we have to get access to the original_func
                assert rf.original_func is not None
                return rf.original_func(contract) & arg(contract)

            return wrap_layer
        else:
            raise ValueError(
                "Applied to wrong type! Maybe you're missing a decorator in the stack..."
            )

    # cast it to a RequireFunction to return it
    rf = cast(RequireFunction[ContractType], inner)
    tag(rf, "require")
    rf.original_func = arg
    return rf


def threshold(
    n: int, l: List[RequireFunction[ContractType]]
) -> RequireFunction[ContractType]:
    """
    threshold takes combinations of length N of conditions from the provided
    list and allows any such group to satisfy.
    """
    if not len(l) >= n:
        raise ValueError("Expected to get more conditions in threshold")
    if not n > 0:
        raise ValueError("Threshold int must be positive")

    if any(get_type_tag(i) != "require" for i in l):
        raise ValueError("All conditions must be require declarations")

    def wrapper(x: RequireWrappable[ContractType]) -> RequireFunction[ContractType]:
        # evaluate each clause 1x
        @require
        def inner(y: ContractType) -> Clause:
            clauses: List[Clause] = [(req.original_func or satisfied)(y) for req in l]
            return Threshold(n, clauses)

        return inner

    c = cast(RequireFunction[ContractType], wrapper)
    tag(c, "require")

    return c


R = TypeVar("R")


# TODO: remove enable_if
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
