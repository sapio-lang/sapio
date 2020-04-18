from typing import TypeVar, Any, Union, Callable, List

from sapio.contract import Contract
from sapio.script.clause import AndClauseArgument
from sapio.txtemplate import TransactionTemplate

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


def path(arg: Union[Callable[[T2], AndClauseArgument], Callable[[T], TransactionTemplate], None] = None)\
        -> Union[Callable[[Any], PathFunction], PathFunction]:
    if arg is None or (hasattr(arg, "__name__") and arg.__name__ == "<lambda>"):
        def wrapper(f: Callable[[T], TransactionTemplate]):
            return PathFunction(f, arg)
        return wrapper
    else:
        return PathFunction(arg, None)


class UnlockFunction():
    # TODO: Improve arg type, which we know is an AndClauseArugment Callable or None
    def __init__(self, condition: Callable[[T], AndClauseArgument], name):
        self.unlock_with = condition
        self.__name__ = name
    def __call__(self, *args, **kwargs):
        return self.unlock_with(*args, **kwargs)


def unlock(s: Callable[[Any], AndClauseArgument]):
    def wrapper(f: Callable[[T], List[Contract]]):
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