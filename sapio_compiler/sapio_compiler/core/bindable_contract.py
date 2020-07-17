from __future__ import annotations

import copy
from abc import abstractmethod
from typing import (
    Any,
    Callable,
    Dict,
    Final,
    Generic,
    List,
    Protocol,
    Tuple,
    Type,
    TypeVar,
    final,
    runtime_checkable,
)
from bitcoin_script_compiler import WitnessManager, Clause
from sapio_bitcoinlib.messages import COutPoint, CTransaction, CTxInWitness, CTxWitness
from sapio_bitcoinlib import miniscript
from sapio_bitcoinlib.static_types import Amount
from sapio_bitcoinlib.script import CScript
from sapio_compiler.core.initializer import Initializer

from .txtemplate import TransactionTemplate
from .amountrange import AmountRange

FieldsType = TypeVar("FieldsType")


class BindableContractProtocol(Protocol):
    @abstractmethod
    @classmethod
    def create_instance(cls, **kwargs: Any) -> BindableContract[Any]:
        pass

    @abstractmethod
    def to_json(self) -> Dict[str, Any]:
        pass

    @abstractmethod
    def bind(
        self, out_in: COutPoint
    ) -> Tuple[List[CTransaction], List[Dict[str, Any]]]:
        pass

    amount_range: AmountRange



class BindableContract(Generic[FieldsType], BindableContractProtocol):
    """
    BindableContract is the base contract object that gets created by the Sapio
    language frontend.

    It should not be directly constructed, but indirectly by inheritance
    through Contract.
    """

    # These slots will be extended later on
    __slots__ = (
        "amount_range",
        "guaranteed_txns",
        "suggested_txns",
        "txn_abi",
        "conditions_abi",
        "witness_manager",
        "fields",
        "is_initialized",
        "init_class",
    )
    witness_manager: WitnessManager
    guaranteed_txns: List[TransactionTemplate]
    suggested_txns: List[TransactionTemplate]
    txn_abi: Dict[Callable, List[TransactionTemplate]]
    conditions_abi: Dict[Callable, Clause]
    amount_range: AmountRange
    fields: FieldsType
    is_initialized: bool
    init_class: Initializer[FieldsType]

    class Fields:
        """
        Fields should be overridden by base classes
        """

        pass

    class MetaData:
        """
        MetaData may be overridden by base classes. It's only used for pretty
        outputs generation so it's not critical that it be set.
        """

        color: Callable[[Any], str] = lambda self: "brown"
        label: Callable[[Any], str] = lambda self: "generic"

    def __getattr__(self, attr: str) -> Any:
        return self.fields.__getattribute__(attr)

    def __setattr__(self, attr: str, v: Any) -> None:
        if attr in self.__slots__:
            super().__setattr__(attr, v)
        elif not self.is_initialized:
            if not hasattr(self, attr):
                raise AssertionError(f"No Known field for {attr} = {v!r}")
            # TODO Type Check
            setattr(self.fields, attr, v)
        else:
            raise AssertionError(
                "Assigning a value to a field is probably a mistake! ", attr
            )

    def __init__(self, **kwargs: Any):
        self.is_initialized = False
        self.txn_abi = {}
        self.conditions_abi = {}
        self.fields: FieldsType = self.__class__.init_class.make_new_fields()
        self.__class__.init_class(self, kwargs)
        self.is_initialized = True

    @final
    @classmethod
    def create_instance(cls, **kwargs: Any) -> BindableContract[FieldsType]:
        return cls(**kwargs)

    @final
    def to_json(self) -> Dict[str, Any]:
        return {
            "witness_manager": self.witness_manager.to_json(),
            "transactions": [
                transaction.to_json()
                for transaction in self.guaranteed_txns + self.suggested_txns
            ],
            "min_amount_spent": self.amount_range.get_min(),
            "max_amount_spent": self.amount_range.get_max(),
            "metadata": {
                "color": self.MetaData.color(self),
                "label": self.MetaData.label(self),
            },
        }

    @final
    def bind(
        self, out_in: COutPoint
    ) -> Tuple[List[CTransaction], List[Dict[str, Any]]]:
        """
        Attaches a BindableContract to a specific COutPoint and generates all
        the child transactions along with metadata entries
        """
        # todo: Note that if a contract has any secret state, it may be a hack
        # attempt to bind it to an output with insufficient funds

        txns = []
        metadata = []
        queue = [(out_in, self)]
        while queue:
            out, this = queue.pop()
            color = this.MetaData.color(this)
            contract_name = this.MetaData.label(this)
            program = this.witness_manager.program
            for (is_ctv, templates) in [
                (True, this.guaranteed_txns),
                (False, this.suggested_txns),
            ]:
                for txn_template in templates:
                    ctv_hash = txn_template.get_ctv_hash() if is_ctv else None

                    # This uniquely binds things with a CTV hash to the
                    # appropriate witnesses. Also binds things with None to all
                    # possible witnesses that do not have a ctv
                    ctv_sat = (miniscript.SatType.TXTEMPLATE, ctv_hash)
                    candidates = [
                        wit for wit in this.witness_manager.ms.sat if ctv_sat in wit
                    ]
                    # There should always be a candidate otherwise we shouldn't
                    # have a txn
                    if not candidates:
                        raise AssertionError("There must always be a candidate")

                    # todo: find correct witness?
                    tx_label = contract_name + ":" + txn_template.label
                    tx = txn_template.bind_tx(out)
                    tx.wit = CTxWitness()
                    tx.wit.vtxinwit.append(CTxInWitness())
                    # Create all possible candidates
                    for wit in candidates:
                        t = copy.deepcopy(tx)
                        t.wit.vtxinwit[0].scriptWitness.stack = wit + [
                            (miniscript.SatType.DATA, program)
                        ]
                        txns.append(t)
                        utxo_metadata = [
                            md.to_json() for md in txn_template.outputs_metadata
                        ]
                        metadata.append(
                            {
                                "color": color,
                                "label": tx_label,
                                "utxo_metadata": utxo_metadata,
                            }
                        )
                    txid = int(tx.hash or tx.rehash(), 16)
                    for (i, (_, contract)) in enumerate(txn_template.outputs):
                        # TODO: CHeck this is correct type into COutpoint
                        queue.append((COutPoint(txid, i), contract))

        return txns, metadata


@runtime_checkable
class ContractProtocol(Protocol[FieldsType]):
    Fields: Type[Any]

    @abstractmethod
    def create_instance(self, **kwargs: Any) -> BindableContract[FieldsType]:
        pass
