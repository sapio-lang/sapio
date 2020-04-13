from __future__ import annotations
from typing import TypeVar
import typing
from lang import *
from functools import singledispatch, wraps

T = TypeVar("T")


import types
def path(arg: Union[Optional[str], Callable[[T], TransactionTemplate]] = None):
    if arg.__name__ == "<lambda>" or arg is None:
        def wrapper(f: Callable[[T], TransactionTemplate]):
            f.is_path = True
            f.unlock_with = arg
            return f
        return wrapper
    else:
        arg.is_path = True
        arg.unlock_with = None
        return arg


def unlock(s: Optional[str] = None) -> object:
    def wrapper(f: Callable[[T], List[Contract]]):
        if hasattr(f, "is_condition") and f.is_condition:
            f.unlock_with = OrClause(f.unlock_with, s)
        else:
            f = classmethod(f)
            f.is_condition = True
            f.unlock_with = s
        return f

    return wrapper

class WithinFee:
    fee_modifier = 100
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



import struct
from bitcoinlib.messages import CTransaction, CTxIn, CTxOut
class TransactionTemplate:
    def __init__(self) -> None:
        self.n_inputs: int = 0
        self.sequences: List[Sequence] = []
        self.outputs: List[(Amount, Contract)] = []
        self.version: Version = Version(u32(c_uint32(2)))
        self.lock_time: LockTime = LockTime(u32(c_uint32(0)))

    def get_ctv_hash(self):
        # Implicitly always at index 0!
        return self.get_standard_template_hash(0)

    def get_standard_template_hash(self, nIn):
        tx = CTransaction()
        tx.nVersion = self.version.value
        tx.nLockTime = self.lock_time.value
        self.vin = [CTxIn(None, None, sequence.value) for sequence in self.sequences]
        self.vouts = [CTxOut(a,b.scriptPubKey) for (a,b) in self.outputs]
        return tx.get_standard_template_hash(nIn)

    def add_output(self, amount, contract):
        WithinFee(contract, amount)
        HasEnoughFunds(contract, amount)
        self.outputs.append((amount, contract))
    def total_amount(self):
        return sum(a for (a,_) in self.outputs)


import inspect
class ExtraArgumentError(AssertionError): pass
class MissingArgumentError(AssertionError): pass
class MetaContract(type):
    def __init__(cls, name, bases, dct):
        super().__init__(cls, name, bases)
        variables = typing.get_type_hints(cls.Fields)
        params = [inspect.Parameter("self", inspect.Parameter.POSITIONAL_ONLY)] + \
                 [inspect.Parameter(param,
                                    inspect.Parameter.KEYWORD_ONLY,
                                    annotation=type_)
                  for param, type_ in variables.items()]
        path_funcs = [v for (k,v) in cls.__dict__.items() if hasattr(v, 'is_path')]
        unlock_funcs = [v for (k,v) in cls.__dict__.items() if hasattr(v, 'is_condition')]
        def init_class(self, **kwargs: Any):
            if kwargs.keys() != variables.keys():
                for key in variables:
                    if key not in kwargs:
                        raise MissingArgumentError("Missing Argument: Keyword arg {} missing".format(key))
                for key in kwargs:
                    if key not in variables:
                        raise ExtraArgumentError("Extra Argument: Key '{}' not in {}".format(key, variables.keys()))
            for key in kwargs:
                # todo: type check here?
                if isinstance(kwargs[key], Variable):
                    setattr(self, key, kwargs[key])
                else:
                    setattr(self, key, Variable(key, kwargs[key]))

            paths = []
            self.amount_range = [21e6 * 100e6, 0]
            for func in path_funcs:
                txn = func(self)
                amount = txn.total_amount()
                self.amount_range = [min(self.amount_range[0], amount),
                                     max(self.amount_range[1], amount)]
                ctv_hash = txn.get_ctv_hash()
                ctv = CheckTemplateVerifyClause(Variable(ctv_hash, ctv_hash))
                paths.append(ctv)
                if func.unlock_with is not None:
                    unlock_clause: Clause = func.unlock_with(self)
                    paths[-1] = AndClause(paths[-1], unlock_clause)
            for func in unlock_funcs:
                paths.append(func.unlock_with(self))

            # prepare for passing to the API...
            # TODO: this gets undone immediately, so maybe
            # provide interface to skip it
            if not paths:
                raise AssertionError("Must Have at least one spending condition")
            while len(paths) > 1:
                p = paths.pop()
                paths[0] = OrClause(paths[-1], p)
            self.scriptPubKey, self.witnesses = ProgramBuilder().compile(paths[0])
            print("\nContract:")
            print(repr(self.scriptPubKey))
            print(self.witnesses)

        sig = inspect.signature(init_class)
        init_class.__signature__ = inspect.Signature(params)
        init_class.__annotations__ = variables.copy()
        setattr(cls, "__init__", init_class)


class Contract(metaclass=MetaContract):
    class Fields:
        pass
    def __init__(self, **kwargs:Any):
        pass

