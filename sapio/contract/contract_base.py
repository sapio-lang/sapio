from __future__ import annotations

import typing
from typing import (
    Any,
    Dict,
    Generator,
    Generic,
    Iterable,
    List,
    Optional,
    Tuple,
    Type,
    TypeVar,
    Union,
)

import sapio.contract.bindable_contract
import sapio.contract.contract
from sapio.bitcoinlib.static_types import Amount, Hash, Sats
from sapio.contract.errors import ExtraArgumentError, MissingArgumentError
from sapio.script.clause import (
    CheckTemplateVerifyClause,
    Clause,
    SatisfiedClause,
    UnsatisfiableClause,
)
from sapio.script.compiler import ProgramBuilder
from sapio.script.variable import AssignedVariable
from sapio.script.witnessmanager import CTVHash, WitnessManager

from .decorators import CheckFunction, PathFunction, PayAddress, UnlockFunction
from .txtemplate import TransactionTemplate

T = TypeVar("T")
FieldsType = TypeVar("FieldsType")


class ContractBase(Generic[FieldsType]):
    ContractType = TypeVar("ContractType", bound="sapio.contract.bindable_contract.BindableContract[FieldsType]")

    def __init__(
        self,
        fields: Type[FieldsType],
        path_functions: List[PathFunction[ContractType]],
        pay_functions: List[PayAddress[ContractType]],
        unlock_functions: List[UnlockFunction[ContractType]],
        assertions: List[CheckFunction[ContractType]],
    ):
        if len(pay_functions):
            assert len(pay_functions) == 1
            assert len(path_functions) == 0
            assert len(unlock_functions) == 0
        self.fields_obj = fields
        self.all_fields: Dict[str, Type[Any]] = typing.get_type_hints(self.fields_obj)
        self.path_functions: List[PathFunction[ContractBase.ContractType]] = path_functions
        self.pay_functions: Optional[PayAddress[ContractBase.ContractType]] = pay_functions[0] if len(
            pay_functions
        ) else None
        self.unlock_functions: List[UnlockFunction[ContractBase.ContractType]] = unlock_functions
        self.assertions: List[CheckFunction[ContractBase.ContractType]] = assertions

    def _setup_call(self, obj: ContractType, kwargs: Dict[str, Any]) -> None:
        if kwargs.keys() != self.all_fields.keys():
            for key in self.all_fields:
                if key not in kwargs:
                    raise MissingArgumentError(
                        "Missing Argument: Keyword arg {} missing from {}".format(
                            key, kwargs.keys()
                        )
                    )
            for key in kwargs:
                if key not in self.all_fields:
                    raise ExtraArgumentError(
                        "Extra Argument: Key '{}' not in {}".format(
                            key, self.all_fields.keys()
                        )
                    )
        for key in kwargs:
            # todo: type check here?
            if isinstance(kwargs[key], AssignedVariable):
                setattr(obj.fields, key, kwargs[key])
            else:
                setattr(obj.fields, key, AssignedVariable(kwargs[key], key))

    def make_new_fields(self) -> Any:
        return self.fields_obj()

    def __call__(
        self,
        obj: ContractBase.ContractType,
        kwargs: Dict[str, Any],
    ) -> None:
        self._setup_call(obj, kwargs)
        obj.amount_range = (Sats(21_000_000 * 100_000_000), Sats(0))
        obj.specific_transactions = []
        if self.pay_functions is not None:
            amt, addr = self.pay_functions(obj)
            # TODO: Something more robust here...
            obj.amount_range = (amt, Amount(0))
            obj.witness_manager = WitnessManager()
            obj.witness_manager.override_program = addr
            return

        # Check all assertions. Assertions should not return anything.
        for assert_func in self.assertions:
            assert_func(obj)

        # Get the value from all paths.
        # Paths return a TransactionTemplate object, or list, or iterable.
        paths: Clause = UnsatisfiableClause()
        for path_func in self.path_functions:
            Ret = Union[typing.Iterator[TransactionTemplate], TransactionTemplate]
            ret: Ret = path_func(obj)
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
                obj.amount_range = (
                    min(obj.amount_range[0], amount),
                    max(obj.amount_range[1], amount),
                )
                ctv_hash = template.get_ctv_hash()
                # TODO: If we OR all the CTV hashes together
                # and then and at the top with the unlock clause,
                # it could help with later code generation sharing the
                # common clause...
                ctv = CheckTemplateVerifyClause(
                    AssignedVariable(Hash(ctv_hash), ctv_hash)
                )
                paths |= ctv & unlock_clause
                obj.specific_transactions.append((CTVHash(ctv_hash), template))
        for unlock_func in self.unlock_functions:
            paths |= unlock_func(obj)

        if isinstance(paths, UnsatisfiableClause):
            raise AssertionError("Must Have at least one spending condition")
        obj.witness_manager = ProgramBuilder().compile(paths)
