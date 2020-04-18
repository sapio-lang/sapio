from __future__ import annotations

import typing
from types import GeneratorType
from typing import Callable, TypeVar, List, Any, Union, Tuple

import sapio.bitcoinlib.hash_functions
from sapio.spending_conditions.script_lang import CheckTemplateVerifyClause, AndClause, OrClause, Variable, \
    AndClauseArgument, SignatureCheckClause
from .bitcoinlib.script import CScript
from .bitcoinlib.static_types import Sequence, Amount, Version, LockTime, uint32, Sats
from sapio.spending_conditions.script_compiler import ProgramBuilder, WitnessManager, CTVHash

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
            raise ValueError("Insufficient Funds", "Contract May Burn Funds!", contract, contract.amount_range, b)


from .bitcoinlib.messages import CTransaction, CTxIn, CTxOut, COutPoint, CTxWitness, CTxInWitness


class MetaDataContainer:
    def __init__(self, color, label):
        self.color = color
        self.label = label
class TransactionTemplate:
    __slots__ = ["n_inputs", "sequences", "outputs", "version", "lock_time", "outputs_metadata"]
    def __init__(self) -> None:
        self.n_inputs: int = 0
        self.sequences: List[Sequence] = [Sequence(uint32(0))]
        self.outputs: List[Tuple[Amount, Contract]] = []
        self.outputs_metadata: List[MetaDataContainer] = []
        self.version: Version = Version(uint32(2))
        self.lock_time: LockTime = LockTime(uint32(0))

    def get_ctv_hash(self):
        # Implicitly always at index 0!
        return self.get_standard_template_hash(0)
    # TODO: Add safety mechanisms here
    def set_sequence(self, sequence:Sequence, idx:int =0):
        self.sequences[idx] = sequence
    def set_locktime(self, sequence:LockTime):
        self.lock_time = sequence

    def get_base_transaction(self) -> CTransaction:
        tx = CTransaction()
        tx.nVersion = self.version
        tx.nLockTime = self.lock_time
        tx.vin = [CTxIn(None, b"", sequence) for sequence in self.sequences]
        tx.vout = [CTxOut(a, b.witness_manager.get_p2wsh_script()) for (a, b) in self.outputs]
        return tx
    def bind_tx(self, point:COutPoint) -> CTransaction:
        tx = self.get_base_transaction()
        tx.vin[0].prevout = point
        tx.rehash()
        return tx


    def get_standard_template_hash(self, nIn):
        return self.get_base_transaction().get_standard_template_hash(nIn)

    def add_output(self, amount : Amount, contract):
        WithinFee(contract, amount)
        HasEnoughFunds(contract, amount)
        self.outputs.append((amount, contract))
        self.outputs_metadata.append(MetaDataContainer(contract.MetaData.color(contract), contract.MetaData.label(contract)))

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

        nmspc['__slots__'] = ('amount_range', 'specific_transactions', 'witness_manager') + tuple(fields.keys())
        params = [inspect.Parameter("self", inspect.Parameter.POSITIONAL_ONLY)] + \
                 [inspect.Parameter(param,
                                    inspect.Parameter.KEYWORD_ONLY,
                                    annotation=type_)
                  for param, type_ in fields.items()]
        pay_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PayAddress)]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]
        if len(pay_funcs):
            assert len(pay_funcs) == 1
            assert len(path_funcs) == 0
            assert len(unlock_funcs) == 0

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
            if len(pay_funcs):
                amt, addr = pay_funcs[0](self)
                self.amount_range = [amt, 0]
                self.witness_manager = WitnessManager()
                self.witness_manager.override_program = addr
                self.specific_transactions = []
                return

            paths : List[AndClauseArgument] = []
            self.amount_range = [Sats(21_000_000 * 100_000_000), Sats(0)]

            self.specific_transactions = []
            for func in assertions:
                func(self)
            for func in path_funcs:
                name = func.__name__
                ret : Union[typing.Iterator[TransactionTemplate], TransactionTemplate] = func(self)
                txns : typing.Iterator[TransactionTemplate]
                if isinstance(ret, TransactionTemplate):
                    txns = iter([ret])
                elif isinstance(ret, GeneratorType):
                    txns = ret
                unlock_clause: typing.Optional[AndClauseArgument] = func.unlock_with(self) if func.unlock_with is not None else None
                for txn in txns:
                    amount = txn.total_amount()
                    self.amount_range = [min(self.amount_range[0], amount),
                                         max(self.amount_range[1], amount)]
                    ctv_hash = txn.get_ctv_hash()
                    ctv = CheckTemplateVerifyClause(Variable(ctv_hash, ctv_hash))
                    # TODO: If we OR all the CTV hashes together
                    # and then and at the top with the unlock clause,
                    # it could help with later code generation sharing the
                    # common clause...
                    paths.append(ctv if unlock_clause is None else AndClause(unlock_clause, ctv))
                    self.specific_transactions.append((CTVHash(ctv_hash), txn))
            for func in unlock_funcs:
                paths.append(func(self))

            # prepare for passing to the API...
            # TODO: this gets undone immediately, so maybe
            # provide interface to skip it
            if not paths:
                raise AssertionError("Must Have at least one spending condition")
            while len(paths) > 1:
                p = paths.pop()
                paths[0] = OrClause(paths[0], p)
            self.witness_manager = ProgramBuilder().compile(paths[0])



        sig = inspect.signature(init_class)
        init_class.__signature__ = inspect.Signature(params)
        nmspc["__init__"] = init_class
        return super(MetaContract, mcl).__new__(mcl, name, bases, nmspc)

def final(m):
    m.__is_final_method__ = True
    return m
import copy
class Contract(metaclass=MetaContract):
    # These slots will be extended later on
    __slots__ = ('amount_range', 'specific_transactions', 'witness_manager')
    witness_manager: WitnessManager
    specific_transactions: typing.Tuple[CTVHash, TransactionTemplate]
    amount_range: Tuple[Amount, Amount]
    class Fields:
        pass
    class MetaData:
        color = lambda self: "brown"
        label = lambda self: "generic"
    # Null __init__ defined to supress sanitizer complaints...
    def __init__(self, **kwargs: Any):
        pass

    @final
    def bind(self, out: COutPoint):
        # todo: Note that if a contract has any secret state, it may be a hack
        # attempt to bind it to an output with insufficient funds
        color = self.MetaData.color(self)
        label = self.MetaData.label(self)

        txns = []
        metadata = []
        for (ctv_hash, txn_template) in self.specific_transactions:
            # todo: find correct witness?
            assert ctv_hash == txn_template.get_ctv_hash()

            tx = txn_template.bind_tx(out)
            txid = tx.sha256
            candidates = [wit for wit in self.witness_manager.witnesses.values() if wit.ctv_hash == ctv_hash]
            # Create all possible candidates
            for wit in candidates:
                t = copy.deepcopy(tx)
                witness = CTxWitness()
                in_witness = CTxInWitness()
                witness.vtxinwit.append(in_witness)
                in_witness.scriptWitness.stack.append(self.witness_manager.program)
                in_witness.scriptWitness.stack.extend(wit.witness)
                t.wit = witness
                txns.append(t)
                utxo_metadata = [{'color': md.color, 'label': md.label} for md in txn_template.outputs_metadata]
                metadata.append(
                    {'color':color, 'label': label, 'utxo_metadata': utxo_metadata})
            for (idx, (_, contract)) in enumerate(txn_template.outputs):
                new_txns, new_metadata = contract.bind(COutPoint(txid, idx))
                txns.extend(new_txns)
                metadata.extend(new_metadata)
        return txns, metadata


