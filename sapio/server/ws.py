from functools import singledispatch

import tornado
import tornado.websocket
import json
from typing import Dict, Type, List, Tuple, Callable, Any
import typing

from sapio.bitcoinlib.static_types import Amount, Sequence, PubKey
from sapio.contract import Contract
import sapio
import sapio.examples.basic_vault
import sapio.examples.p2pk
from sapio.spending_conditions.script_lang import TimeSpec, RelativeTimeSpec, Variable

typeconv = {
    Amount : "int",
    Sequence: "int",
    TimeSpec: "int",
    PubKey: "String",
    Contract: [0, "String"],
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
    return bytes(b, 'utf-8')
@register(Contract)
def convert(arg: Contract,ctx):
    if arg[1] in ctx.compilation_cache:
        return ctx.compilation_cache[arg[1]]
    return sapio.examples.p2pk.PayToSegwitAddress(amount=arg[0], address=arg[1])

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

    def on_message(self, message):
        request = json.loads(message)
        if request['type'] == "create":
            create_req = request['content']
            type_ = create_req['type']
            if type_ in self.menu:
                args = create_req['args']
                args_t = self.menu[type_]
                conv_args = self.conv[type_]
                if args.keys() != args_t.keys():
                    self.close()
                for (name, value) in args.items():
                    typ = args_t[name]
                    args[name] = conv_args[name](value, self)
                print("ARGS", args)
                contract = self.contracts[type_](**args)
                addr = contract.witness_manager.get_p2wsh_address()
                amount = contract.amount_range[1]
                self.compilation_cache[addr] = contract
                self.write_message(
                    {"type": "created", 'content': [int(amount), addr]}
                )

        elif request['type'] == "close":
            self.close()
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
            if hint in typeconv:
                menu[key] = typeconv[hint]
                conv[key] = conversion_functions[hint]
            else:
                print(key, str(hint))
                assert False
        cls.menu[name] = menu
        cls.conv[name] = conv
        cls.contracts[name] = contract
        cls.cached = None


def make_app():
    return tornado.web.Application([
        (r"/", CompilerWebSocket),
    ])

if __name__ == "__main__":
    CompilerWebSocket.add_contract("p2pk", sapio.examples.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("vault", sapio.examples.basic_vault.Vault2)
    app = make_app()
    app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
