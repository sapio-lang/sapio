from __future__ import annotations

import typing
from collections.abc import Iterable
from types import GeneratorType
from typing import List, Any, Union, Tuple

from sapio.script.clause import CheckTemplateVerifyClause, Variable, \
    AndClauseArgument, UnsatisfiableClause, SatisfiedClause
from sapio.script.compiler import ProgramBuilder
from sapio.bitcoinlib.messages import COutPoint, CTxWitness, CTxInWitness
from sapio.bitcoinlib.static_types import Amount, Sats
from sapio.script.witnessmanager import CTVHash, WitnessManager
from .txtemplate import TransactionTemplate
from .decorators import PathFunction, UnlockFunction, PayAddress, CheckFunction, final
from .errors import ExtraArgumentError, MissingArgumentError




import inspect


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
                        raise MissingArgumentError(
                            "Missing Argument: Keyword arg {} missing from {}".format(key, kwargs.keys()))
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

            paths: AndClauseArgument = UnsatisfiableClause()
            self.amount_range = [Sats(21_000_000 * 100_000_000), Sats(0)]

            self.specific_transactions = []
            for func in assertions:
                func(self)
            for func in path_funcs:
                ret: Union[typing.Iterator[TransactionTemplate], TransactionTemplate] = func(self)
                txns: typing.Iterator[TransactionTemplate]
                if isinstance(ret, TransactionTemplate):
                    txns = iter([ret])
                elif isinstance(ret, (GeneratorType, Iterable)):
                    txns = ret
                else:
                    raise ValueError("Invalid Return Type", ret)
                unlock_clause: AndClauseArgument = SatisfiedClause()
                if func.unlock_with is not None:
                    unlock_clause = func.unlock_with(self)
                for txn in txns:
                    txn.label = func.__name__
                    amount = txn.total_amount()
                    self.amount_range = [min(self.amount_range[0], amount),
                                         max(self.amount_range[1], amount)]
                    ctv_hash = txn.get_ctv_hash()
                    ctv = CheckTemplateVerifyClause(Variable(ctv_hash, ctv_hash))
                    # TODO: If we OR all the CTV hashes together
                    # and then and at the top with the unlock clause,
                    # it could help with later code generation sharing the
                    # common clause...
                    paths = (ctv & unlock_clause) | paths
                    self.specific_transactions.append((CTVHash(ctv_hash), txn))
            for func in unlock_funcs:
                paths = paths | func(self)

            # prepare for passing to the API...
            # TODO: this gets undone immediately, so maybe
            # provide interface to skip it
            if paths is UnsatisfiableClause:
                raise AssertionError("Must Have at least one spending condition")
            self.witness_manager = ProgramBuilder().compile(paths)

        sig = inspect.signature(init_class)
        init_class.__signature__ = inspect.Signature(params)
        nmspc["__init__"] = init_class
        return super(MetaContract, mcl).__new__(mcl, name, bases, nmspc)


import copy


class Contract(metaclass=MetaContract):
    # These slots will be extended later on
    __slots__ = ('amount_range', 'specific_transactions', 'witness_manager')
    witness_manager: WitnessManager
    specific_transactions: List[typing.Tuple[CTVHash, TransactionTemplate]]
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
        output_label = self.MetaData.label(self)

        txns = []
        metadata = []
        for (ctv_hash, txn_template) in self.specific_transactions:
            # todo: find correct witness?
            assert ctv_hash == txn_template.get_ctv_hash()
            tx_label = output_label + ":" + txn_template.label

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
                    {'color': color, 'label': tx_label, 'utxo_metadata': utxo_metadata})
            for (idx, (_, contract)) in enumerate(txn_template.outputs):
                new_txns, new_metadata = contract.bind(COutPoint(txid, idx))
                txns.extend(new_txns)
                metadata.extend(new_metadata)
        return txns, metadata
