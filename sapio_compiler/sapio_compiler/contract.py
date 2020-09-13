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
    class X(ContractBase[Props], ContractProtocol[Props]):
        f""" Base Class for Contract {in_name}"""
        Props: ClassVar[Type[Props]] = props_t
        f""" Interior {in_name} State Type"""
        # Class Variables
        then_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        finish_or_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        finish_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]] = []
        assert_funcs: ClassVar[List[Callable[[Props], bool]]] = []
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

    # Wrap as a new_class to rename
    return types.new_class(in_name, bases=(X,))


def contract(props_t_in: Type[Any]) -> Type[ContractProtocol[Any]]:
    props_t = dataclass(props_t_in)
    traits = getattr(props_t, "Traits", [])
    name = getattr(props_t, "__OVERRIDE_NAME__", props_t.__name__)
    return MakeContract(name, props_t, traits)
