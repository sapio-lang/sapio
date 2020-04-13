from __future__ import annotations

import typing

from .lang import *

T = TypeVar("T")


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


from .bitcoinlib.messages import CTransaction, CTxIn, CTxOut, COutPoint


class TransactionTemplate:
    __slots__ = ["n_inputs", "sequences", "outputs", "version", "lock_time"]
    def __init__(self) -> None:
        self.n_inputs: int = 0
        self.sequences: List[Sequence] = [Sequence(u32(c_uint32(0)))]
        self.outputs: List[(Amount, Contract)] = []
        self.version: Version = Version(u32(c_uint32(2)))
        self.lock_time: LockTime = LockTime(u32(c_uint32(0)))

    def get_ctv_hash(self):
        # Implicitly always at index 0!
        return self.get_standard_template_hash(0)

    def get_base_transaction(self) -> CTransaction:
        tx = CTransaction()
        tx.nVersion = self.version.value
        tx.nLockTime = self.lock_time.value
        tx.vin = [CTxIn(None, b"", sequence.value) for sequence in self.sequences]
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

    def add_output(self, amount, contract):
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
        path_funcs = [v for (k, v) in nmspc.items() if hasattr(v, 'is_path')]
        unlock_funcs = [v for (k, v) in nmspc.items() if hasattr(v, 'is_condition')]

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

            paths = []
            self.amount_range = [21e6 * 100e6, 0]

            self.transactions = {}
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
    class Fields:
        pass
    # Null __init__ defined to supress sanitizer complaints...
    def __init__(self, **kwargs: Any):
        pass

    @final
    def clear(self): pass

    @final
    def bind(self, out: COutPoint):
        txns = []
        witnesses_by_name = {wit.nickname:wit.witness for wit in self.witnesses}
        for (_, child) in self.transactions.items():
            # todo: find correct witness?
            name = child.get_ctv_hash()
            if name in witnesses_by_name:
                print(witnesses_by_name[name])
                # Todo: Incorrect type because we can't fill in things like signatures!
                tx = child.bind_tx(out, witnesses_by_name[name])
            else:
                tx = child.bind_tx(out, CTxWitness())
            print(repr(tx))
            txid = tx.sha256
            txns.append(tx)
            for (idx, (_, contract)) in enumerate(child.outputs):
                txns.extend(contract.bind(COutPoint(txid, idx)))
        return txns

