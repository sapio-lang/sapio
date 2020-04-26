
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