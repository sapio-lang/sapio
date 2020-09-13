from bitcoin_script_compiler import (
    WitnessManager,
    Clause,
)
from sapio_bitcoinlib.static_types import Amount, Hash, Sats
from sapio_bitcoinlib.script import CScript
from typing import (
    Any,
    Dict,
    List,
    Type,
    ClassVar,
    Callable,
    Tuple,
    Optional,
)
from .core.txtemplate import TransactionTemplate
from .core.amountrange import AmountRange
from .core.protocol import (
    ContractProtocol,
    ContractBase,
    IndexType,
    Props,
    Trait,
    ThenFuncIndex,
    FinishOrFuncIndex,
    FinishFuncIndex,
    FuncIndex,
    ThenF,
    Finisher,
    TxRetType,
)
import types
from dataclasses import dataclass


Contract = ContractProtocol[Any]


def MakeContract(
    in_name: str,
    props_t: Type[Props],
    traits: List[Trait],
) -> Type[ContractProtocol[Props]]:
    # Get a Global ContractFactory
    global ContractFactory

    class ContractFactory(ContractBase[Props], ContractProtocol[Props]):
        class Props(props_t):
            f""" Interior {in_name} State Type"""
        class Then:
            """Continuations driven by OP_CHECKTEMPLATEVERIFY"""
        class Finish:
            f"""End of {in_name} with key signature or other satisfaction"""
        class FinishOr:
            f"""End of {in_name} with key signature or other satisfaction,
            and additional logic to suggest a next transaction.
            """
        class Requires:
            f"""Properties required by {in_name} for correctness"""
        class Let:
            f"""{in_name} Bindings for specific reusable logic clauses"""

        # Class Variables
        _then_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        _finish_or_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        _finish_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        _assert_funcs: ClassVar[List[Callable[[Props], bool]]] = []
        override: Optional[Callable[[Props], Tuple[AmountRange, str]]] = None

        # Instance Variables
        data: Props
        f""" Interior {in_name} State Type"""
        txn_abi: Dict[str, Tuple[ThenF[Props], List[TransactionTemplate]]]
        conditions_abi: Dict[str, Tuple[ThenF[Props], Clause]]
        witness_manager: WitnessManager
        amount_range: AmountRange

        def __init__(self, data: Props) -> None:
            super().__init__(data)

    ContractFactory.__doc__ = props_t.__doc__
    ContractFactory.__name__ = in_name
    ContractFactory.__module__ = props_t.__module__
    # Wrap as a new_class to rename
    Y = types.new_class(in_name, (ContractFactory,))
    Y.__module__ = ContractFactory.__module__
    Y.__doc__ = props_t.__doc__
    # don't leak the ContractFactory ref
    del ContractFactory
    return Y


def contract(props_t_in: Type[Any]) -> Type[ContractProtocol[Any]]:
    props_t = dataclass(props_t_in)
    traits = getattr(props_t, "Traits", [])
    name = getattr(props_t, "__OVERRIDE_NAME__", props_t.__name__)
    return MakeContract(name, props_t, traits)
