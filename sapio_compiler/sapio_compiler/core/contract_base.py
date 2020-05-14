"""
contract_base.py
-----------------

Initialization routine for a new contract instance.

"""
from __future__ import annotations

import typing
from typing import (
    Any,
    Callable,
    Dict,
    Generator,
    Generic,
    Iterable,
    List,
    Optional,
    Type,
    TypeVar,
    Tuple,
    Union,
)

import sapio_compiler.contract
import sapio_compiler.core.bindable_contract
from bitcoin_script_compiler import (
    AssignedVariable,
    CheckTemplateVerifyClause,
    Clause,
    CTVHash,
    ProgramBuilder,
    SatisfiedClause,
    UnsatisfiableClause,
    WitnessManager,
)
from bitcoinlib.static_types import Amount, Hash, Sats
from sapio_compiler.core.errors import ExtraArgumentError, MissingArgumentError

from .txtemplate import TransactionTemplate
from sapio_compiler.decorators import (
    PayFunction,
    CheckFunction,
    UnlockFunction,
    PathFunction,
)

T = TypeVar("T")
FieldsType = TypeVar("FieldsType")


class ContractBase(Generic[FieldsType]):
    """
    ContractBase handles the initialization logic of a a new instance of a contract.

    For performance, as much pre-processing as possible is done in __init__ of the ContractBase.
    """

    ContractType = TypeVar(
        "ContractType",
        bound="sapio_compiler.core.bindable_contract.BindableContract[FieldsType]",
    )

    def __init__(
        self,
        fields: Type[FieldsType],
        path_functions: List[PathFunction[ContractType]],
        pay_functions: List[PayFunction[ContractType]],
        unlock_functions: List[UnlockFunction[ContractType]],
        assertions: List[CheckFunction[ContractType]],
    ):
        if len(pay_functions):
            assert len(pay_functions) == 1
            assert len(path_functions) == 0
            assert len(unlock_functions) == 0
        self.fields_obj = fields
        self.all_fields: Dict[str, Type[Any]] = typing.get_type_hints(self.fields_obj)
        self.path_functions = path_functions
        self.pay_functions: Optional[
            PayFunction[ContractBase.ContractType]
        ] = pay_functions[0] if len(pay_functions) else None
        self.unlock_functions = unlock_functions
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

    def __call__(self, obj: ContractBase.ContractType, kwargs: Dict[str, Any],) -> None:
        self._setup_call(obj, kwargs)
        obj.amount_range = sapio_compiler.core.bindable_contract.AmountRange()
        obj.guaranteed_txns = []
        obj.suggested_txns = []
        # Check all assertions. Assertions should not return anything.
        for assert_func in self.assertions:
            if not assert_func(obj):
                raise AssertionError(
                    f"CheckFunction for {obj.__name__} did not throw any error, but returned False"
                )
        if self.pay_functions is not None:
            amt_rng, addr = self.pay_functions(obj)
            obj.amount_range = amt_rng
            obj.witness_manager = WitnessManager()
            obj.witness_manager.override_program = addr
            return

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
                template.finalize()
                template.label = path_func.__name__
                amount = template.total_amount()
                obj.amount_range.update_range(amount)
                # not all transactions are guaranteed
                if path_func.is_guaranteed:
                    # ctv_hash is an identifier and a txid equivalent
                    ctv_hash = template.get_ctv_hash()
                    # TODO: If we OR all the CTV hashes together
                    # and then and at the top with the unlock clause,
                    # it could help with later code generation sharing the
                    ctv = CheckTemplateVerifyClause(
                        AssignedVariable(Hash(ctv_hash), ctv_hash)
                    )
                    paths |= ctv & unlock_clause
                    obj.guaranteed_txns.append(template)
                else:
                    paths |= unlock_clause
                    obj.suggested_txns.append(template)
        for unlock_func in self.unlock_functions:
            paths |= unlock_func(obj)

        if isinstance(paths, UnsatisfiableClause):
            raise AssertionError("Must Have at least one spending condition")
        obj.witness_manager = ProgramBuilder().compile(paths)
