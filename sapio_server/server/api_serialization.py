import json
import typing
from typing import Dict, Type, Callable, Any, Union, Tuple, Optional, List

import tornado
import tornado.websocket

import sapio_zoo.p2pk
from bitcoinlib import segwit_addr
from bitcoinlib.messages import COutPoint
from bitcoinlib.static_types import Amount, Sequence, PubKey
from sapio_compiler import Contract
from sapio_zoo.tree_pay import TreePay
from sapio_zoo.undo_send import UndoSend2
from bitcoin_script_compiler.clause import (
    TimeSpec,
    RelativeTimeSpec,
    AbsoluteTimeSpec,
    Days,
)
from bitcoinlib.static_types import int64

from sapio_compiler import BindableContract

placeholder_hint = {
    Amount: 0,
    Sequence: "int",
    Union[RelativeTimeSpec, AbsoluteTimeSpec]: "int",
    RelativeTimeSpec: "int",
    AbsoluteTimeSpec: "int",
    typing.List[typing.Tuple[Amount, Contract]]: [[0, [0, "address"]]],
    PubKey: "String",
    Contract: [0, "String"],
    sapio_zoo.p2pk.PayToSegwitAddress: "Address",
    int: 0,
}
id = lambda x: x


def convert_pubkey(arg: str, ctx) -> PubKey:
    return PubKey(bytes(arg, "utf-8"))


def convert_contract_object(arg: Tuple[Amount, str], ctx) -> Contract:
    try:
        return ctx.compilation_cache[arg[1]]
    except KeyError:
        return sapio_zoo.p2pk.PayToSegwitAddress(amount=arg[0], address=arg[1])
        # raise AssertionError("No Known Contract by that name")


def convert_dest(arg: List[Tuple[int, str]], ctx) -> List[Tuple[Amount, Contract]]:
    return list(map(lambda x: convert_contract(x, ctx), arg))


def convert_contract(arg: Tuple[int, str], ctx) -> Tuple[Amount, Contract]:
    try:
        return (Amount(arg[0]), ctx.compilation_cache[arg[1]])
    except KeyError:
        return (
            Amount(arg[0]),
            sapio_zoo.p2pk.PayToSegwitAddress(amount=Amount(arg[0]), address=arg[1]),
        )


# Don't convert to p2swa if we know what it is... TODO: maybe make this optional?
def convert_p2swa(arg: str, ctx) -> Contract:
    try:
        return ctx.compilation_cache[arg]
    except KeyError:
        # default bind to 0
        return sapio_zoo.p2pk.PayToSegwitAddress(amount=10000, address=arg)


def convert_sequence(arg: Sequence, ctx) -> Sequence:
    return Sequence(arg)


def convert_relative_time_spec(arg: Any, ctx) -> RelativeTimeSpec:
    return RelativeTimeSpec(Sequence(arg))


def convert_amount(arg: int, ctx) -> Amount:
    # TODO Assert ranges....
    return Amount(int64(arg))


conversion_functions: Dict[Type, Callable] = {
    PubKey: convert_pubkey,
    Contract: convert_contract_object,
    List[Tuple[Amount, Contract]]: convert_dest,
    Tuple[Amount, Contract]: convert_contract,
    Amount: convert_amount,
    Sequence: convert_sequence,
    RelativeTimeSpec: convert_relative_time_spec,
    int: lambda x, y: x,
    str: lambda x, y: x,
    Union[AbsoluteTimeSpec, RelativeTimeSpec]: lambda x: RelativeTimeSpec(Sequence(x)),
    sapio_zoo.p2pk.PayToSegwitAddress: convert_p2swa,
}
