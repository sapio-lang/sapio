import copy
import typing
from typing import List, Tuple

from sapio.bitcoinlib.messages import COutPoint, CTxWitness, CTxInWitness
from sapio.bitcoinlib.static_types import Amount
from .txtemplate import TransactionTemplate
from .decorators import final
from sapio.script.witnessmanager import WitnessManager, CTVHash


class BindableContract:
    # These slots will be extended later on
    __slots__ = ('amount_range', 'specific_transactions', 'witness_manager')
    witness_manager: WitnessManager
    specific_transactions: List[typing.Tuple[CTVHash, TransactionTemplate]]
    amount_range: Tuple[Amount, Amount]
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