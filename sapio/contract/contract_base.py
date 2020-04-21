from __future__ import annotations
import typing
from typing import Dict, Any, List, Optional, Tuple, Union, Generator, Iterable

from sapio.bitcoinlib.static_types import Amount, Sats
from .txtemplate import  TransactionTemplate
from .decorators import PathFunction, PayAddress, UnlockFunction, CheckFunction
from sapio.contract.errors import MissingArgumentError, ExtraArgumentError
from sapio.script.clause import Clause, UnsatisfiableClause, SatisfiedClause, CheckTemplateVerifyClause
from sapio.script.compiler import ProgramBuilder
from sapio.script.variable import AssignedVariable
from sapio.script.witnessmanager import WitnessManager, CTVHash
import sapio.contract.contract


class ContractBase:
    def __init__(self, fields: Dict[str, Any], path_functions: List[PathFunction], pay_functions: List[PayAddress],
                 unlock_functions: List[UnlockFunction], assertions: List[CheckFunction]):
        if len(pay_functions):
            assert len(pay_functions) == 1
            assert len(path_functions) == 0
            assert len(unlock_functions) == 0
        self.fields = fields
        self.path_functions: List[PathFunction] = path_functions
        self.pay_functions: Optional[PayAddress] = pay_functions[0] if len(pay_functions) else None
        self.unlock_functions: List[UnlockFunction] = unlock_functions
        self.assertions: List[CheckFunction] = assertions

    def _setup_call(self, obj:sapio.contract.contract.Contract, kwargs: Dict[str, Any]):
        if kwargs.keys() != self.fields.keys():
            for key in self.fields:
                if key not in kwargs:
                    raise MissingArgumentError(
                        "Missing Argument: Keyword arg {} missing from {}".format(key, kwargs.keys()))
            for key in kwargs:
                if key not in self.fields:
                    raise ExtraArgumentError("Extra Argument: Key '{}' not in {}".format(key, self.fields.keys()))

        for key in kwargs:
            # todo: type check here?
            if isinstance(kwargs[key], AssignedVariable):
                setattr(obj, key, kwargs[key])
            else:
                setattr(obj, key, AssignedVariable(kwargs[key], key))

    def __call__(self, obj, **kwargs: Dict[str, Any]):
        self._setup_call(obj, kwargs)
        obj.amount_range = (Sats(21_000_000 * 100_000_000), Sats(0))
        obj.specific_transactions = []
        if self.pay_functions is not None:
            amt, addr = self.pay_functions(obj)
            # TODO: Something more robust here...
            obj.amount_range = (amt, 0)
            obj.witness_manager = WitnessManager()
            obj.witness_manager.override_program = addr
            return


        # Check all assertions. Assertions should not return anything.
        for assert_func in self.assertions: assert_func(obj)

        # Get the value from all paths.
        # Paths return a TransactionTemplate object, or list, or iterable.
        paths: Clause = UnsatisfiableClause()
        for path_func in self.path_functions:
            T = Union[typing.Iterator[TransactionTemplate], TransactionTemplate]
            ret: T = path_func(obj)
            transaction_templates: typing.Iterator[TransactionTemplate]
            if isinstance(ret, TransactionTemplate):
                # Wrap value for uniform handling below
                transaction_templates = iter([ret])
            elif isinstance(ret, (Generator, Iterable)):
                transaction_templates = ret
            else:
                raise ValueError("Invalid Return Type", ret)
            unlock_clause: Clause = SatisfiedClause()
            if path_func.unlock_with is not None:
                unlock_clause = path_func.unlock_with(obj)
            for template in transaction_templates:
                template.label = path_func.__name__
                amount = template.total_amount()
                obj.amount_range = (min(obj.amount_range[0], amount),
                                     max(obj.amount_range[1], amount))
                ctv_hash = template.get_ctv_hash()
                # TODO: If we OR all the CTV hashes together
                # and then and at the top with the unlock clause,
                # it could help with later code generation sharing the
                # common clause...
                ctv = CheckTemplateVerifyClause(AssignedVariable(ctv_hash, ctv_hash))
                paths |= (ctv & unlock_clause)
                obj.specific_transactions.append((CTVHash(ctv_hash), template))
        for unlock_func in self.unlock_functions:
            paths |= unlock_func(obj)

        if paths is UnsatisfiableClause:
            raise AssertionError("Must Have at least one spending condition")
        obj.witness_manager = ProgramBuilder().compile(paths)