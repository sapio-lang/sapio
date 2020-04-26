from __future__ import annotations

from typing import List, Tuple, Dict, Any

from sapio.bitcoinlib.static_types import uint32

import sapio.contract
from sapio.bitcoinlib.messages import CTransaction, CTxIn, CTxOut, COutPoint
from sapio.bitcoinlib.static_types import Sequence, Amount, Version, LockTime
from sapio.contract.assertions import WithinFee, HasEnoughFunds

import sapio.contract.bindable_contract

class MetaDataContainer:
    def __init__(self, color:str, label:str) -> None:
        self.color : str = color
        self.label: str = label
    def to_json(self) -> Dict[str, str]:
        return {
            "color": self.color,
            "label": self.label,
        }


class TransactionTemplate:
    __slots__ = ["n_inputs", "sequences", "outputs", "version", "lock_time", "outputs_metadata", "label"]

    def __init__(self) -> None:
        self.n_inputs: int = 0
        self.sequences: List[Sequence] = [Sequence(uint32(0))]
        self.outputs: List[Tuple[Amount, sapio.contract.bindable_contract.BindableContract[Any]]] = []
        self.outputs_metadata: List[MetaDataContainer] = []
        self.version: Version = Version(uint32(2))
        self.lock_time: LockTime = LockTime(uint32(0))
        self.label: str = ""

    def to_json(self) -> Dict[str, Any]:
        return {
            "n_inputs": self.n_inputs,
            "sequences": self.sequences,
            "version": self.version,
            "lock_time": self.lock_time,
            "label": self.label,
            "outputs": [(amt, contract.to_json()) for (amt, contract) in self.outputs],
            "outputs_metadata": [o.to_json() for o in self.outputs_metadata]
        }

    def get_ctv_hash(self) -> bytes:
        # Implicitly always at index 0!
        return self.get_standard_template_hash(0)

    # TODO: Add safety mechanisms here
    def set_sequence(self, sequence: Sequence, idx: int = 0) -> None:
        self.sequences[idx] = sequence

    def set_locktime(self, sequence: LockTime) -> None:
        self.lock_time = sequence

    def get_base_transaction(self) -> CTransaction:
        tx = CTransaction()
        tx.nVersion = self.version
        tx.nLockTime = self.lock_time
        tx.vin = [CTxIn(None, b"", sequence) for sequence in self.sequences]
        tx.vout = [CTxOut(a, b.witness_manager.get_p2wsh_script()) for (a, b) in self.outputs]
        return tx

    def bind_tx(self, point: COutPoint) -> CTransaction:
        tx = self.get_base_transaction()
        tx.vin[0].prevout = point
        tx.rehash()
        return tx

    def get_standard_template_hash(self, nIn: int) -> bytes:
        return self.get_base_transaction().get_standard_template_hash(nIn)

    def add_output(self, amount: Amount, contract: sapio.contract.bindable_contract.BindableContract[Any]) -> None:
        WithinFee(contract, amount)
        HasEnoughFunds(contract, amount)
        self.outputs.append((amount, contract))
        self.outputs_metadata.append(
            MetaDataContainer(contract.MetaData.color(contract), contract.MetaData.label(contract)))

    def total_amount(self) -> Amount:
        return Amount(sum(a for (a, _) in self.outputs))
