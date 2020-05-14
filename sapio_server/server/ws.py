import json
import typing
from typing import Any, Callable, Dict, Optional, Tuple, Type, Union

import tornado
import tornado.websocket

from bitcoinlib import segwit_addr
from bitcoinlib.messages import COutPoint, CTransaction, CTxIn, CTxOut
from bitcoinlib.static_types import Amount, PubKey, Sequence
from sapio_compiler import BindableContract, ContractProtocol
from sapio_compiler import Contract

from .api_serialization import conversion_functions, placeholder_hint

DEBUG = True


base_tx = CTransaction()
base_tx.vin.append(CTxIn())
base_tx.vout.append(CTxOut())
base_tx.rehash()
base_out = COutPoint(base_tx.sha256, 0)
base_meta = {
    "color": "white",
    "label": "Base Contract Unspecified",
    "utxo_metadata": [],
}


class CompilerWebSocket(tornado.websocket.WebSocketHandler):
    contracts: Dict[str, Union[BindableContract, ContractProtocol]] = {}
    menu: Dict[str, Dict[str, str]] = {}
    conv: Dict[str, Dict[str, Callable[[Any], Any]]] = {}
    cached: Optional[str] = None
    compilation_cache: Dict[str, BindableContract]
    example_message: Any = None

    @classmethod
    def set_example(cls, example: BindableContract):
        txns, metadata = example.bind(base_out)
        txns.append(base_tx)
        metadata.append(base_meta)
        addr = example.witness_manager.get_p2wsh_address()
        amount = example.amount_range.max
        data = [
            {"hex": tx.serialize_with_witness().hex(), **meta}
            for (tx, meta) in zip(txns, metadata)
        ]
        cls.example_message = {
            "type": "created",
            "content": [int(amount), addr, {"program": data}],
        }

    def open(self):
        if self.cached is None:
            print(self.menu)
            cached = json.dumps({"type": "menu", "content": self.menu})
        self.write_message(cached)
        if self.example_message is not None:
            self.write_message(self.example_message)
        self.compilation_cache = {}

    """
    Start/End Protocol:
    # Server enumerates available Contract Blocks and their arguments
        Server: {type: "menu", content: {contract_name : {arg_name: data type, ...}, ...}}
        Server: {type: "session_id", content: [bool, String]}
        ...
        Client: {type: "close"}

    Create Contract:
    # Attempt to create a Contract
    # Contract may access a compilation cache of both saved and not saved Contracts
        Client: {type: "create", content: {type: contract_name, {arg_name:data, ...}...}}
        Server: {type: "created", content: [Amount, Address]}

    Save Contract:
    # Attempt to save Contract to durable storage for this session
    # If session id was [false, _] should not return true (but may!)
        Client: {type: "save", content: Address}
        Server: {type: "saved", content: Bool}

    Export Session:
    # Provide a JSON of all saved data for this session
        Client: {type: "export"}
        Server: {type: "exported", content: ...}

    Export Authenticated:
    # Provide a signed Pickle object which can be re-loaded
    # directly if the signature checks
        Client: {type: "export_auth"}
        Server: {type: "exported_auth", content: ...}

    Load Authenticated:
    # Provide a signed Pickle object which can be re-loaded
    # directly if the signature checks to the current session
        Client: {type: "load_auth", content:...}
        Server: {type: "loaded_auth", content: bool}

    Bind Contract:
    # Attach a Contract to a particular UTXO
    # Return all Transactions
        Client: {type: "bind", content: [COutPoint, Address]}
        Server: {type: "bound", content: [Transactions]}



    """

    def on_message(self, message):
        print()
        print("#####################")
        print("New Message:")
        print(message)
        request = json.loads(message)
        print(request)
        request_type = request["type"]
        if request_type == "create":
            create_req = request["content"]
            create_type = create_req["type"]
            if create_type in self.menu:
                args = create_req["args"]
                args_t = self.menu[create_type]
                conv_args = self.conv[create_type]
                if args.keys() != args_t.keys():
                    if not DEBUG:
                        self.close()
                    else:
                        print("Mismatch", args, args_t)
                for (name, value) in args.items():
                    typ = args_t[name]
                    args[name] = conv_args[name](value, self)
                print("ARGS", args)
                contract = self.contracts[create_type].create_instance(**args)
                addr = contract.witness_manager.get_p2wsh_address()
                amount = contract.amount_range.max
                self.compilation_cache[addr] = contract
                txns, metadata = contract.bind(base_out)
                txns.append(base_tx)
                metadata.append(base_meta)
                data = [
                    {"hex": tx.serialize_with_witness().hex(), **meta}
                    for (tx, meta) in zip(txns, metadata)
                ]
                self.write_message(
                    {
                        "type": "created",
                        "content": [int(amount), addr, {"program": data}],
                    }
                )
        elif request_type == "bind":
            raise NotImplementedError("Pending!")
        elif request_type == "load_auth":
            raise NotImplementedError("Pending!")
        elif request_type == "export_auth":
            raise NotImplementedError("Pending!")
        elif request_type == "export":
            raise NotImplementedError("Pending!")
        elif request_type == "save":
            raise NotImplementedError("Pending!")
        elif request_type == "close":
            self.close()
        else:
            if DEBUG:
                print("No Type", request_type)
            else:
                self.close()

    def on_close(self):
        print("WebSocket closed")

    @classmethod
    def add_contract(cls, name: str, contract: Any):
        assert isinstance(contract, (BindableContract, ContractProtocol))
        assert name not in cls.menu
        hints = typing.get_type_hints(contract.Fields)
        menu: Dict[str, Any] = {}
        conv: Dict[str, Callable] = {}
        for key, hint in hints.items():
            if hint in placeholder_hint:
                menu[key] = placeholder_hint[hint]
                conv[key] = conversion_functions[hint]
            else:
                print(key, str(hint))
                assert False
        cls.menu[name] = menu
        cls.conv[name] = conv
        cls.contracts[name] = contract
        cls.cached = None

    def check_origin(self, origin):
        allowed = ["http://localhost:3000", "http://localhost:5000"]
        if origin in allowed:
            print("allowed", origin)
            return 1
