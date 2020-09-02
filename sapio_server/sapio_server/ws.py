import json
import typing
from typing import Any, Callable, Dict, Optional, Tuple, Type, Union, ClassVar

import tornado
import tornado.websocket

from sapio_bitcoinlib import segwit_addr
from sapio_bitcoinlib import miniscript
from sapio_bitcoinlib.messages import COutPoint, CTransaction, CTxIn, CTxOut
from sapio_bitcoinlib.static_types import Amount, PubKey, Sequence
from sapio_compiler import Contract, ContractProtocol
from sapio_compiler import Contract

from .api_serialization import conversion_functions, Context, create_jsonschema
import jsonschema

DEBUG = True


base_tx = CTransaction()
base_tx.vout.append(CTxOut())
base_tx.rehash()
from hashlib import sha256
base_out = COutPoint(int(sha256(b"mock:"+bytes(f"{0}", 'utf-8')).digest().hex(), 16), 0)

allowed_sat_types = {
    miniscript.SatType.SIGNATURE,
    miniscript.SatType.KEY_AND_HASH160_PREIMAGE,
    miniscript.SatType.SHA256_PREIMAGE,
    miniscript.SatType.HASH256_PREIMAGE,
    miniscript.SatType.RIPEMD160_PREIMAGE,
    miniscript.SatType.HASH160_PREIMAGE,
    miniscript.SatType.DATA
}


def clean_witness(tx):
    # TODO: Store the witness satisfaction templates in a different format to
    # avoid creating invalid CTransaction objects.
    for witness in tx.wit.vtxinwit:
        witness.scriptWitness.stack = [w[1] for w in witness.scriptWitness.stack if w[0] in allowed_sat_types]
    return tx


def get_tx_data(txns, metadata):
    return [
        {"hex": clean_witness(tx).serialize_with_witness().hex(), **meta}
        for (tx, meta) in zip(txns, metadata)
    ]


class CompilerWebSocket(tornado.websocket.WebSocketHandler):
    contracts: Dict[str, Union[Contract, ContractProtocol]] = {}
    menu_items: ClassVar = []
    menu_items_map: ClassVar = {}
    menu: ClassVar = {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "oneOf": menu_items,
    }
    deserialize_args: Dict[str, Dict[str, Type]] = {}
    cached: Optional[str] = None
    example_message: Any = None
    context: Context

    @classmethod
    def set_example(cls, example: Contract):
        txns, metadata = example.bind(base_out)
        addr = example.witness_manager.get_p2wsh_address()
        amount = example.amount_range.max
        data = get_tx_data(txns, metadata)
        cls.example_message = {
            "action": "created",
            "content": [int(amount), addr, {"program": data}],
        }

    def open(self):
        if self.cached is None:
            cached = json.dumps({"action": "menu", "content": self.menu})
        self.write_message(cached)
        if self.example_message is not None:
            self.write_message(self.example_message)
        self.context = Context()

    """
    Start/End Protocol:
    # Server enumerates available Contract Blocks and their arguments
        Server: {action: "menu", content: {contract_name : {arg_name: data type, ...}, ...}}
        Server: {action: "session_id", content: [bool, String]}
        ...
        Client: {action: "close"}

    Create Contract:
    # Attempt to create a Contract
    # Contract may access a compilation cache of both saved and not saved Contracts
        Client: {action: "create", content: {type: contract_name, {arg_name:data, ...}...}}
        Server: {action: "created", content: [Amount, Address]}

    Save Contract:
    # Attempt to save Contract to durable storage for this session
    # If session id was [false, _] should not return true (but may!)
        Client: {action: "save", content: Address}
        Server: {action: "saved", content: Bool}

    Export Session:
    # Provide a JSON of all saved data for this session
        Client: {action: "export"}
        Server: {action: "exported", content: ...}

    Export Authenticated:
    # Provide a signed Pickle object which can be re-loaded
    # directly if the signature checks
        Client: {action: "export_auth"}
        Server: {action: "exported_auth", content: ...}

    Load Authenticated:
    # Provide a signed Pickle object which can be re-loaded
    # directly if the signature checks to the current session
        Client: {action: "load_auth", content:...}
        Server: {action: "loaded_auth", content: bool}

    Bind Contract:
    # Attach a Contract to a particular UTXO
    # Return all Transactions
        Client: {action: "bind", content: [COutPoint, Address]}
        Server: {action: "bound", content: [Transactions]}
    """
    PROTOCOL_SCHEMA = {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "action": {"const": "create"},
                    "content": {
                        "type": "object",
                        "properties": {
                            "type": {"type": "string"},
                            "args": {"type": "object"},
                        },
                        "required": ["type", "args"],
                    },
                },
                "required": ["content"],
            }
        ],
        "required": ["action"],
    }

    def on_message(self, raw_message):
        toplevel_request = json.loads(raw_message)
        # TODO: Cache!
        jsonschema.Draft7Validator(CompilerWebSocket.PROTOCOL_SCHEMA).validate(
            toplevel_request
        )

        action = toplevel_request["action"]
        if action == "create":
            request = toplevel_request["content"]
            contract_type = request["type"]
            args = request["args"]
            self.menu_items_map[contract_type].validate(args)
            deserialize_args = self.deserialize_args[contract_type]
            contract = self.contracts[contract_type].create_instance(
                **{
                    name: conversion_functions[deserialize_args[name]](
                        value, self.context
                    )
                    for (name, value) in args.items()
                }
            )
            addr = contract.witness_manager.get_p2wsh_address()
            amount = contract.amount_range.max
            self.context.cache(addr, contract)
            txns, metadata = contract.bind(base_out)
            data = get_tx_data(txns, metadata)
            self.write_message(
                {
                    "action": "created",
                    "content": [int(amount), addr, {"program": data}],
                }
            )
        elif action == "bind":
            raise NotImplementedError("Pending!")
        elif action == "load_auth":
            raise NotImplementedError("Pending!")
        elif action == "export_auth":
            raise NotImplementedError("Pending!")
        elif action == "export":
            raise NotImplementedError("Pending!")
        elif action == "save":
            raise NotImplementedError("Pending!")
        elif action == "close":
            self.close()
        else:
            if DEBUG:
                print("No Type", action)
            else:
                self.close()

    def on_close(self):
        print("WebSocket closed")

    @classmethod
    def add_contract(cls, name: str, contract: Any):
        assert isinstance(contract, (Contract, ContractProtocol))
        assert name not in cls.menu
        hints = typing.get_type_hints(contract.Fields)
        menu: Dict[str, Any] = create_jsonschema(name, hints.items())
        cls.menu_items.append(menu)
        cls.menu_items_map[name] = jsonschema.Draft7Validator(menu)
        cls.deserialize_args[name] = hints
        cls.contracts[name] = contract
        cls.cached = None

    def check_origin(self, origin):
        allowed = ["http://localhost:3000", "http://localhost:5000"]
        if origin in allowed:
            print("allowed", origin)
            return 1
