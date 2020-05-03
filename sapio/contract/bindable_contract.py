from __future__ import annotations

import copy
import typing
from abc import abstractmethod
from typing import (
    final,
    Any,
    Callable,
    Dict,
    Generic,
    List,
    Optional,
    Protocol,
    Tuple,
    Type,
    TypeVar,
    runtime_checkable,
)


from sapio.bitcoinlib.messages import COutPoint, CTransaction, CTxInWitness, CTxWitness
from sapio.bitcoinlib.static_types import Amount
from sapio.contract.contract_base import ContractBase
from sapio.script.variable import AssignedVariable
from sapio.script.witnessmanager import CTVHash, WitnessManager

from .txtemplate import TransactionTemplate

T = TypeVar("T")


class BindableContract(Generic[T]):
    # These slots will be extended later on
    __slots__ = (
        "amount_range",
        "guaranteed_txns",
        "suggested_txns",
        "witness_manager",
        "fields",
        "is_initialized",
        "init_class",
    )
    witness_manager: WitnessManager
    guaranteed_txns: List[TransactionTemplate]
    suggested_txns: List[TransactionTemplate]
    amount_range: Tuple[Amount, Amount]
    fields: T
    is_initialized: bool
    init_class: ContractBase[T]

    class Fields:
        pass

    class MetaData:
        color: Callable[[Any], str] = lambda self: "brown"
        label: Callable[[Any], str] = lambda self: "generic"

    def __getattr__(self, attr: str) -> AssignedVariable[Any]:
        return self.fields.__getattribute__(attr)

    def __setattr__(self, attr: str, v: Any) -> None:
        if attr in self.__slots__:
            super().__setattr__(attr, v)
        elif not self.is_initialized:
            if not hasattr(self, attr):
                raise AssertionError("No Known field for " + attr + " = " + repr(v))
            # TODO Type Check
            setattr(self.fields, attr, v)
        else:
            raise AssertionError(
                "Assigning a value to a field is probably a mistake! ", attr
            )

    def __init__(self, **kwargs: Any):
        self.is_initialized = False
        self.fields: T = self.__class__.init_class.make_new_fields()
        self.__class__.init_class(self, kwargs)
        self.is_initialized = True

    @final
    @classmethod
    def create_instance(cls, **kwargs: Any) -> BindableContract[T]:
        return cls(**kwargs)

    @final
    def to_json(self) -> Dict[str, Any]:
        return {
            "witness_manager": self.witness_manager.to_json(),
            "transactions": [
                transaction.to_json()
               for transaction in self.guaranteed_txns+self.suggested_txns
            ],
            "min_amount_spent": self.amount_range[0],
            "max_amount_spent": self.amount_range[1],
            "metadata": {
                "color": self.MetaData.color(self),
                "label": self.MetaData.label(self),
            },
        }

    @final
    def bind(self, out: COutPoint) -> Tuple[List[CTransaction], List[Dict[str, Any]]]:
        # todo: Note that if a contract has any secret state, it may be a hack
        # attempt to bind it to an output with insufficient funds
        color = self.MetaData.color(self)
        output_label = self.MetaData.label(self)

        txns = []
        metadata = []
        for (has_witness, templates) in [
            (True, self.guaranteed_txns),
            (False, self.suggested_txns),
        ]:
            for txn_template in templates:
                # todo: find correct witness?
                tx_label = output_label + ":" + txn_template.label
                tx = txn_template.bind_tx(out)
                txid = int(tx.rehash(), 16)
                ctv_hash = txn_template.get_ctv_hash() if has_witness else None

                # This uniquely binds things with a CTV hash to the appropriate witnesses
                # And binds things with None to all possible witnesses.
                candidates = [
                    wit
                    for wit in self.witness_manager.witnesses.values()
                    if wit.ctv_hash == ctv_hash
                ]
                # There should always be a candidate otherwise we shouldn't have a txn
                assert candidates
                # Create all possible candidates
                for wit in candidates:
                    t = copy.deepcopy(tx)
                    witness = CTxWitness()
                    in_witness = CTxInWitness()
                    witness.vtxinwit.append(in_witness)
                    in_witness.scriptWitness.stack.extend(wit.witness)
                    in_witness.scriptWitness.stack.append(self.witness_manager.program)
                    t.wit = witness
                    txns.append(t)
                    utxo_metadata = [
                        {"color": md.color, "label": md.label}
                        for md in txn_template.outputs_metadata
                    ]
                    metadata.append(
                        {
                            "color": color,
                            "label": tx_label,
                            "utxo_metadata": utxo_metadata,
                        }
                    )
                for (idx, (_, contract)) in enumerate(txn_template.outputs):
                    # TODO: CHeck this is correct type into COutpoint
                    new_txns, new_metadata = contract.bind(COutPoint(txid, idx))
                    txns.extend(new_txns)
                    metadata.extend(new_metadata)

        return txns, metadata


@runtime_checkable
class ContractProtocol(Protocol[T]):
    Fields: Type[Any]

    @abstractmethod
    def create_instance(self, **kwargs: Any) -> BindableContract[T]:
        pass
