from __future__ import annotations
from typing import TypeVar, Any, Union, Callable, List
import sapio
import sapio.contract
from sapio.script.clause import Clause

T = TypeVar("T")
T2 = TypeVar("T2")


class PathFunction():
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, f: Any, arg: Any):
        self.f = f
        self.unlock_with = arg
        self.__name__ = f.__name__
    def __call__(self, *args, **kwargs):
        return self.f(*args, **kwargs)


def path(arg: Union[Callable[[T2], Clause], Callable[[T], sapio.contract.TransactionTemplate], None] = None)\
        -> Union[Callable[[Any], PathFunction], PathFunction]:
    if arg is None or (hasattr(arg, "__name__") and arg.__name__ == "<lambda>"):
        def wrapper(f: Callable[[T], sapio.contract.TransactionTemplate]):
            return PathFunction(f, arg)
        return wrapper
    else:
        return PathFunction(arg, None)


class UnlockFunction():
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[T], Clause], name):
        self.unlock_with = condition
        self.__name__ = name
    def __call__(self, *args, **kwargs):
        return self.unlock_with(*args, **kwargs)


def unlock(s: Callable[[Any], Clause]):
    def wrapper(f: Callable[[T], List[sapio.contract.Contract]]):
        return UnlockFunction(s, f.__name__)
    return wrapper


class PayAddress():
    def __init__(self, address):
        self.address = address
    def __call__(self, *args, **kwargs):
        return self.address(*args, **kwargs)


def pay_address(f):
    return PayAddress(f)


class CheckFunction():
    def __init__(self, func):
        self.func = func
        self.__name__ = func.__name__
    def __call__(self, *args, **kwargs):
        self.func(*args, **kwargs)


def check(s: Callable[[T], bool]) -> Callable[[T], bool]:
    return CheckFunction(s)

def final(m):
    m.__is_final_method__ = True
    return m
