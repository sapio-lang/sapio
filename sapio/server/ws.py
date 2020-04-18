from functools import singledispatch

import tornado
import tornado.websocket
import json
from typing import Dict, Type, List, Tuple, Callable, Any
import typing

from sapio.bitcoinlib.messages import COutPoint
from sapio.bitcoinlib.static_types import Amount, Sequence, PubKey
from sapio.contract import Contract
import sapio
import sapio.examples.basic_vault
import sapio.examples.p2pk
import sapio.examples.subscription
from sapio.spending_conditions.script_lang import TimeSpec, RelativeTimeSpec, Variable, AbsoluteTimeSpec

placeholder_hint = {
    Amount : "int",
    Sequence: "int",
    TimeSpec: "int",
    RelativeTimeSpec: "int",
    AbsoluteTimeSpec: "int",
    PubKey: "String",
    Contract: [0, "String"],
    sapio.examples.p2pk.PayToSegwitAddress: "Address",
    int: "int",
}
id = lambda x: x

conversion_functions = {}
def register(type_):
    def deco(f):
        conversion_functions[type_] = f
        return f
    return deco


@register(PubKey)
def convert(arg: PubKey,ctx):
    return bytes(arg, 'utf-8')
@register(Contract)
def convert(arg: Contract,ctx):
    if arg[1] in ctx.compilation_cache:
        return ctx.compilation_cache[arg[1]]
    return sapio.examples.p2pk.PayToSegwitAddress(amount=arg[0], address=arg[1])

@register(sapio.examples.p2pk.PayToSegwitAddress)
def convert(arg: Contract,ctx):
    if arg in ctx.compilation_cache:
        return ctx.compilation_cache[arg]
    return sapio.examples.p2pk.PayToSegwitAddress(amount=0, address=arg)

@register(Sequence)
@register(RelativeTimeSpec)
@register(TimeSpec)
def convert(arg: Sequence, ctx):
    return (RelativeTimeSpec(Sequence(arg)))
@register(Amount)
@register(Sequence)
@register(int)
def id(x, ctx):
    return x

DEBUG = True
class CompilerWebSocket(tornado.websocket.WebSocketHandler):
    contracts: Dict[str, Type[Contract]] = {}
    menu: Dict[str, Dict[str, str]]= {}
    conv: Dict[str, Dict[str, Callable[[Any], Any]]]= {}
    cached :str = None
    compilation_cache : Dict[str, Contract] = None
    def open(self):
        if self.cached is None:
            print(self.menu)
            cached = json.dumps({"type": "menu", "content":self.menu})
        self.write_message(cached)
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
        request_type = request['type']
        if request_type == "create":
            create_req = request['content']
            create_type = create_req['type']
            if create_type in self.menu:
                args = create_req['args']
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
                contract = self.contracts[create_type](**args)
                addr = contract.witness_manager.get_p2wsh_address()
                amount = contract.amount_range[1]
                self.compilation_cache[addr] = contract
                txns, metadata = contract.bind(COutPoint())
                data = [{'hex':tx.serialize_with_witness().hex(), **meta} for (tx, meta) in zip(txns, metadata)]
                self.write_message(
                    {"type": "created", 'content': [int(amount), addr, {'program':data}]}
                )
        elif request_type == "bind": raise NotImplementedError('Pending!')
        elif request_type == "load_auth": raise NotImplementedError('Pending!')
        elif request_type == "export_auth": raise NotImplementedError('Pending!')
        elif request_type == "export": raise NotImplementedError('Pending!')
        elif request_type == "save": raise NotImplementedError('Pending!')
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
    def add_contract(cls, name:str, contract:Type[Contract]):
        assert name not in cls.menu
        hints = typing.get_type_hints(contract.Fields)
        menu = {}
        conv = {}
        for key,hint in hints.items():
            if hint == sapio.examples.subscription.Hide:
                continue
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


def make_app():
    return tornado.web.Application([
        (r"/", CompilerWebSocket),
    ], autoreload=True)

if __name__ == "__main__":
    CompilerWebSocket.add_contract("Pay to Public Key", sapio.examples.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Vault", sapio.examples.basic_vault.Vault2)
    CompilerWebSocket.add_contract("Subscription", sapio.examples.subscription.auto_pay)
    app = make_app()
    app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
