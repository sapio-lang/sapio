from __future__ import annotations

import typing
from typing import Callable, TypeVar, List, Any, Union, Tuple

import sapio.bitcoinlib.hash_functions
from sapio.script_lang import CheckTemplateVerifyClause, AndClause, OrClause, Variable, AndClauseArgument
from .bitcoinlib.script import CScript
from .bitcoinlib.static_types import Sequence, Amount, Version, LockTime, uint32, Sats
from .script_compiler import ProgramBuilder

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


class CheckFunction():
    def __init__(self, func):
        self.func = func
        self.__name__ = func.__name__
    def __call__(self, *args, **kwargs):
        self.func(*args, **kwargs)

def check(s: Callable[[T], bool]) -> Callable[[T], bool]:
    return CheckFunction(s)


class WithinFee:
    fee_modifier : Amount = Sats(100)

    def __init__(self, contract, b):
        if contract.amount_range[0] + self.fee_modifier < b:
            raise ValueError("Contract May Burn Funds!")

    @classmethod
    def change_fee_modifier(cls, fee_modifier):
        cls.fee_modifier = fee_modifier


class HasEnoughFunds:
    def __init__(self, contract, b):
        if contract.amount_range[1] > b:
            raise ValueError("Contract May Burn Funds!")


from .bitcoinlib.messages import CTransaction, CTxIn, CTxOut, COutPoint, CTxWitness


class TransactionTemplate:
    __slots__ = ["n_inputs", "sequences", "outputs", "version", "lock_time"]
    def __init__(self) -> None:
        self.n_inputs: int = 0
        self.sequences: List[Sequence] = [Sequence(uint32(0))]
        self.outputs: List[Tuple[Amount, Contract]] = []
        self.version: Version = Version(uint32(2))
        self.lock_time: LockTime = LockTime(uint32(0))

    def get_ctv_hash(self):
        # Implicitly always at index 0!
        return self.get_standard_template_hash(0)
    def set_sequence(self, sequence:Sequence, idx:int =0):
        self.sequences[idx] = sequence

    def get_base_transaction(self) -> CTransaction:
        tx = CTransaction()
        tx.nVersion = self.version
        tx.nLockTime = self.lock_time
        tx.vin = [CTxIn(None, b"", sequence) for sequence in self.sequences]
        tx.vout = [CTxOut(a, b.scriptPubKey) for (a, b) in self.outputs]
        return tx
    def bind_tx(self, point:COutPoint, witness:CTxWitness) -> CTransaction:
        tx = self.get_base_transaction()
        tx.vin[0].prevout = point
        tx.wit.vtxinwit.append(witness)
        tx.rehash()
        return tx


    def get_standard_template_hash(self, nIn):
        return self.get_base_transaction().get_standard_template_hash(nIn)

    def add_output(self, amount : Amount, contract):
        WithinFee(contract, amount)
        HasEnoughFunds(contract, amount)
        self.outputs.append((amount, contract))

    def total_amount(self):
        return sum(a for (a, _) in self.outputs)


import inspect


class ExtraArgumentError(AssertionError): pass


class MissingArgumentError(AssertionError): pass


class MetaContract(type):
    def __new__(mcl, name, bases, nmspc):
        fields = typing.get_type_hints(nmspc['Fields'])
        nmspc['__annotations__'] = fields.copy()

        for base in bases:
            for method_name in dir(base):
                method = getattr(base, method_name)
                if hasattr(method, "__is_final_method__") and method.__is_final_method__:
                    if hasattr(method, "__call__"):
                        if method_name in nmspc:
                            raise ValueError("Cannot Override Final Method")
                    else:
                        raise ValueError("Cannot Override Final ???")

        nmspc['__slots__'] = ('amount_range', 'transactions', 'witnesses', 'scriptPubKey') + tuple(fields.keys())
        params = [inspect.Parameter("self", inspect.Parameter.POSITIONAL_ONLY)] + \
                 [inspect.Parameter(param,
                                    inspect.Parameter.KEYWORD_ONLY,
                                    annotation=type_)
                  for param, type_ in fields.items()]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]

        def init_class(self, **kwargs: Any):
            if kwargs.keys() != fields.keys():
                for key in fields:
                    if key not in kwargs:
                        raise MissingArgumentError("Missing Argument: Keyword arg {} missing".format(key))
                for key in kwargs:
                    if key not in fields:
                        raise ExtraArgumentError("Extra Argument: Key '{}' not in {}".format(key, fields.keys()))
            for key in kwargs:
                # todo: type check here?
                if isinstance(kwargs[key], Variable):
                    setattr(self, key, kwargs[key])
                else:
                    setattr(self, key, Variable(key, kwargs[key]))

            paths : List[AndClauseArgument] = []
            self.amount_range = [Sats(21_000_000 * 100_000_000), Sats(0)]

            self.transactions = {}
            for func in assertions:
                func(self)
            for func in path_funcs:
                name = func.__name__
                txn = func(self)
                self.transactions[name] = txn
                amount = txn.total_amount()
                self.amount_range = [min(self.amount_range[0], amount),
                                     max(self.amount_range[1], amount)]
                ctv_hash = txn.get_ctv_hash()
                ctv = CheckTemplateVerifyClause(Variable(ctv_hash, ctv_hash))
                paths.append(ctv)
                if func.unlock_with is not None:
                    unlock_clause: AndClauseArgument = func.unlock_with(self)
                    paths[-1] = AndClause(paths[-1], unlock_clause)
            for func in unlock_funcs:
                paths.append(func(self))

            # prepare for passing to the API...
            # TODO: this gets undone immediately, so maybe
            # provide interface to skip it
            if not paths:
                raise AssertionError("Must Have at least one spending condition")
            while len(paths) > 1:
                p = paths.pop()
                paths[0] = OrClause(paths[-1], p)
            self.scriptPubKey, self.witnesses = ProgramBuilder().compile(paths[0])


        sig = inspect.signature(init_class)
        init_class.__signature__ = inspect.Signature(params)
        nmspc["__init__"] = init_class
        return super(MetaContract, mcl).__new__(mcl, name, bases, nmspc)

def final(m):
    m.__is_final_method__ = True
    return m
class Contract(metaclass=MetaContract):
    # These slots will be extended later on
    __slots__ = ('amount_range', 'transactions', 'witnesses', 'scriptPubKey')
    scriptPubKey: CScript
    witnesses: typing.Dict[Any, Any]
    transactions: typing.Dict[Any, Any]
    amount_range: Tuple[int, int]
    class Fields:
        pass
    # Null __init__ defined to supress sanitizer complaints...
    def __init__(self, **kwargs: Any):
        pass

    @final
    def bind(self, out: COutPoint):
        txns = []
        witnesses_by_name = {wit.nickname:wit.witness for wit in self.witnesses}
        for (_, child) in self.transactions.items():
            # todo: find correct witness?
            name = child.get_ctv_hash()
            if name in witnesses_by_name:
                # Todo: Incorrect type because we can't fill in things like signatures!
                tx = child.bind_tx(out, witnesses_by_name[name])
            else:
                tx = child.bind_tx(out, CTxWitness())
            txid = sapio.bitcoinlib.hash_functions.sha256
            txns.append(tx)
            for (idx, (_, contract)) in enumerate(child.outputs):
                txns.extend(contract.bind(COutPoint(txid, idx)))
        return txns

