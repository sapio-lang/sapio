from __future__ import annotations
class Contract: pass

from typing import TypeVar

import typing
from lang import *


class MetaContract(type):
    def __init__(cls, name, bases, dct):
        super().__init__(cls, name, bases)
        cls.variables = typing.get_type_hints(cls.Fields)

        def __init__(self, **kwargs):
            if len(kwargs) != len(cls.variables):
                raise AssertionError("args {} does not cover {}".format(kwargs.keys(), cls.variables.keys()))
            for key in kwargs:
                if key not in cls.variables:
                    raise AssertionError("Key '{}' not in {}".format(key, cls.variables.keys()))
                setattr(self, key, Variable(key, kwargs[key]) if not isinstance(kwargs[key], Variable) else kwargs[key])

            paths = []
            self.amount_range = [21e6*100e6,0]
            for (k, v) in dct.items():
                if hasattr(v, 'is_path'):
                    txn = getattr(self, k)()
                    amount = txn.total_amount()
                    self.amount_range = [min(self.amount_range[0], amount),
                                         max(self.amount_range[1], amount)]
                    ctv_hash = txn.get_ctv_hash()
                    ctv = CheckTemplateVerifyClause(Variable(ctv_hash, ctv_hash))
                    if v.unlock_with is None:
                        paths.append(ctv)
                    else:
                        unlock_clause: Clause = v.unlock_with(self)
                        paths.append(
                            AndClause(unlock_clause, ctv)
                        )

                if hasattr(v, 'is_condition'):
                    paths.append((v.unlock_with)(self))
            while len(paths) >= 2:
                a = paths.pop()
                b = paths.pop()
                paths.append(OrClause(a, b))
            else:
                if len(paths):
                    paths = paths[0]
                else:
                    paths = None
            self.scriptPubKey, self.witnesses = ProgramBuilder().compile(paths)
            print("\nContract:")
            print(paths)
            print(repr(self.scriptPubKey))
            print((self.scriptPubKey))
            print(type(self.scriptPubKey))
            print(self.witnesses)

        setattr(cls, "__init__", __init__)


T = TypeVar("T")

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


# From Bitcoin Core messages.py test framework
import struct
def ser_compact_size(l):
    r = b""
    if l < 253:
        r = struct.pack("B", l)
    elif l < 0x10000:
        r = struct.pack("<BH", 253, l)
    elif l < 0x100000000:
        r = struct.pack("<BI", 254, l)
    else:
        r = struct.pack("<BQ", 255, l)
    return r

def ser_string(s):
    return ser_compact_size(len(s)) + s


class CTxOut:
    __slots__ = ("nValue", "scriptPubKey")

    def __init__(self, nValue=0, scriptPubKey=b""):
        self.nValue = nValue
        self.scriptPubKey = scriptPubKey

    def serialize(self):
        r = b""
        r += struct.pack("<q", self.nValue)
        r += ser_string(self.scriptPubKey)
        return r

    def __repr__(self):
        return "CTxOut(nValue=%i.%08i scriptPubKey=%s)" \
               % (self.nValue // COIN, self.nValue % COIN,
                  self.scriptPubKey.hex())


import hashlib
def sha256(s):
    return hashlib.new('sha256', s).digest()


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
        r = b""
        r += struct.pack("<i", self.version.value)
        r += struct.pack("<I", self.lock_time.value)
#        if any(inp.scriptSig for inp in self.vin):
#            r += sha256(b"".join(ser_string(inp.scriptSig) for inp in self.vin))
        r += struct.pack("<I", self.n_inputs)
        r += sha256(b"".join(struct.pack("<I", seq.value) for seq in self.sequences))
        r += struct.pack("<I", len(self.outputs))
        outs = [CTxOut(a,b.scriptPubKey) for (a,b) in self.outputs]
        r += sha256(b"".join(out.serialize() for out in outs))
        r += struct.pack("<I", nIn)
        return sha256(r)

    def add_output(self, amount, contract):
        WithinFee(contract, amount)
        HasEnoughFunds(contract, amount)
        self.outputs.append((amount, contract))
    def total_amount(self):
        return sum(a for (a,_) in self.outputs)
