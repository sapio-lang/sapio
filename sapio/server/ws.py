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
typeconvf = {
    PubKey: lambda b: bytes(b, 'utf-8'),
    Contract: lambda x: sapio.examples.p2pk.PayToSegwitAddress(amount=x[0], address=x[1]),
    int: int,
    Amount: int,
    Sequence: lambda x: (RelativeTimeSpec(Sequence(x))),
    TimeSpec: lambda x: (RelativeTimeSpec(Sequence(x)))
}

class CompilerWebSocket(tornado.websocket.WebSocketHandler):
    contracts: Dict[str, Type[Contract]] = {}
    menu: Dict[str, Dict[str, str]]= {}
    conv: Dict[str, Dict[str, Callable[[Any], Any]]]= {}
    cached :str = None
    def open(self):
        if self.cached is None:
            print(self.menu)
            cached = json.dumps({"type": "menu", "content":self.menu})
        self.write_message(cached)

    def on_message(self, message):
        request = json.loads(message)
        if request['type'] == "create":
            req = request['content']
            if req['type'] in self.menu:
                args = req['args']
                args_t = self.menu[req['type']]
                conv_args = self.conv[req['type']]
                if args.keys() != args_t.keys():
                    self.close()
                for (name, value) in args.items():
                    typ = args_t[name]
                    args[name] = conv_args[name](value)
                    # TODO: Type Check/Cast?
                print("ARGS", args)
                contract = self.contracts[req['type']](**args)
                addr = contract.witness_manager.get_p2wsh_address()
                amount = contract.amount_range[1]
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
                conv[key] = typeconvf[hint]
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
