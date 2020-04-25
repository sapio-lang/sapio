import json
import typing
from typing import Dict, Type, Callable, Any, Union, Tuple, Optional

import tornado
import tornado.websocket

import sapio
import sapio.examples.basic_vault
import sapio.examples.p2pk
import sapio.examples.subscription
from sapio.bitcoinlib import segwit_addr
from sapio.bitcoinlib.messages import COutPoint
from sapio.bitcoinlib.static_types import Amount, Sequence, PubKey
from sapio.contract.contract import Contract
from sapio.examples.tree_pay import TreePay
from sapio.examples.undo_send import UndoSend2
from sapio.script.clause import TimeSpec, RelativeTimeSpec, AbsoluteTimeSpec, Days

from sapio.contract.bindable_contract import BindableContract

placeholder_hint = {
    Amount: "int",
    Sequence: "int",
    TimeSpec: "int",
    RelativeTimeSpec: "int",
    AbsoluteTimeSpec: "int",
    typing.List[typing.Tuple[Amount, Contract]]: [[0, [0, "address"]]],
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
def convert(arg: PubKey, ctx):
    return bytes(arg, 'utf-8')


@register(typing.List[typing.Tuple[Amount, Contract]])
def convert_dest(arg, ctx):
    ret = [(convert_amount(Amount(a), ctx), convert_contract(b, ctx)) for (a, b) in arg]
    print(ret)
    return ret


@register(Tuple[Amount, str])
def convert_contract(arg: Tuple[Amount, str], ctx):
    if arg[1] in ctx.compilation_cache:
        return ctx.compilation_cache[arg[1]]
    return sapio.examples.p2pk.PayToSegwitAddress(amount=arg[0], address=arg[1])

@register(Contract)
def convert_contract_object(arg: Contract, ctx):
    if arg.witness_manager.get_p2wsh_address() in ctx.compilation_cache:
        return ctx.compilation_cache[arg[1]]
    raise AssertionError("No Known Contract by that name")


@register(sapio.examples.p2pk.PayToSegwitAddress)
def convert_p2swa(arg: Contract, ctx):
    if arg in ctx.compilation_cache:
        return ctx.compilation_cache[arg]
    return sapio.examples.p2pk.PayToSegwitAddress(amount=0, address=arg)


@register(Sequence)
@register(RelativeTimeSpec)
@register(TimeSpec)
def convert_time(arg: Sequence, ctx):
    return (RelativeTimeSpec(Sequence(arg)))


@register(Amount)
@register(int)
def convert_amount(x, ctx):
    return x


DEBUG = True


class CompilerWebSocket(tornado.websocket.WebSocketHandler):
    contracts: Dict[str, Type[Contract]] = {}
    menu: Dict[str, Dict[str, str]] = {}
    conv: Dict[str, Dict[str, Callable[[Any], Any]]] = {}
    cached: Optional[str] = None
    compilation_cache: Dict[str, BindableContract] = None
    example_message: Any = None

    @classmethod
    def set_example(cls, example: BindableContract):
        txns, metadata = example.bind(COutPoint())
        addr = example.witness_manager.get_p2wsh_address()
        amount = example.amount_range[1]
        data = [{'hex': tx.serialize_with_witness().hex(), **meta} for (tx, meta) in zip(txns, metadata)]
        cls.example_message = {"type": "created", 'content': [int(amount), addr, {'program': data}]}

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
                data = [{'hex': tx.serialize_with_witness().hex(), **meta} for (tx, meta) in zip(txns, metadata)]
                self.write_message(
                    {"type": "created", 'content': [int(amount), addr, {'program': data}]}
                )
        elif request_type == "bind":
            raise NotImplementedError('Pending!')
        elif request_type == "load_auth":
            raise NotImplementedError('Pending!')
        elif request_type == "export_auth":
            raise NotImplementedError('Pending!')
        elif request_type == "export":
            raise NotImplementedError('Pending!')
        elif request_type == "save":
            raise NotImplementedError('Pending!')
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
    def add_contract(cls, name: str, contract: Union[Type[BindableContract], Callable[[Any], BindableContract]]):
        assert name not in cls.menu
        hints = typing.get_type_hints(contract.Fields)
        menu = {}
        conv = {}
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


def make_app():
    return tornado.web.Application([
        (r"/", CompilerWebSocket),
    ], autoreload=True)


import os

if __name__ == "__main__":
    CompilerWebSocket.add_contract("Pay to Public Key", sapio.examples.p2pk.PayToPubKey)
    CompilerWebSocket.add_contract("Vault", sapio.examples.basic_vault.Vault2)
    CompilerWebSocket.add_contract("Subscription", sapio.examples.subscription.auto_pay)
    CompilerWebSocket.add_contract("TreePay", TreePay)
    generate_n_address = [segwit_addr.encode('bcrt', 0, os.urandom(32)) for _ in range(16)]
    payments = [(5, sapio.examples.p2pk.PayToSegwitAddress(amount=0, address=address)) for address in
                generate_n_address]
    example = TreePay(payments=payments, radix=8)
    # amount: Amount
    # recipient: PayToSegwitAddress
    # schedule: List[Tuple[AbsoluteTimeSpec, Amount]]
    # return_address: PayToSegwitAddress
    # watchtower_key: PubKey
    # return_timeout: RelativeTimeSpec

    N_EMPLOYEES = 2
    generate_address = lambda: sapio.examples.p2pk.PayToSegwitAddress(amount=0, address=segwit_addr.encode('bcrt', 0,
                                                                                                           os.urandom(
                                                                                                               32)))
    employee_addresses = [(1, generate_address()) for _ in range(N_EMPLOYEES)]

    import datetime

    now = datetime.datetime.now()
    day = datetime.timedelta(1)
    DURATION = 2
    employee_payments = [(perdiem * DURATION,
                          sapio.examples.subscription.CancellableSubscription(amount=perdiem * DURATION,
                                                                              recipient=address, schedule=[
                                  (AbsoluteTimeSpec.from_date(now + (1 + x) * day), perdiem) for x in range(DURATION)],
                                                                              return_address=generate_address(),
                                                                              watchtower_key=b"",
                                                                              return_timeout=Days(1))) for
                         (perdiem, address) in employee_addresses]
    tree1 = TreePay(payments=employee_payments, radix=2)
    sum_pay = [((amt*DURATION),addr) for (amt, addr) in employee_addresses]
    tree2 = TreePay(payments=sum_pay, radix=2)
    total_amount = sum(x for (x, _) in sum_pay)
    example = UndoSend2(to_contract=tree2, from_contract=tree1, amount=total_amount, timeout=Days(10))

    CompilerWebSocket.set_example(example)
    print(CompilerWebSocket.example_message)

    app = make_app()
    app.listen(8888)
    tornado.ioloop.IOLoop.current().start()
