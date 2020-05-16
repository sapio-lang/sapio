from typing import TypedDict, Union

from jsonschema import Draft7Validator
from server.context import Context
from server.network.json import amount
from sapio_compiler import BindableContract

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {"address": {"type": "string",}, "amount": amount.schema,},
    "required": ["address"],
    "maxProperties": 2,
}
validator = Draft7Validator(schema)


class AmountField(TypedDict, total=False):
    amount: Union[amount.Bitcoin, amount.Satoshis]


class AddrDict(TypedDict, AmountField):
    address: str


def convert(arg: AddrDict, ctx: Context) -> BindableContract:
    validator.validate(arg)

    cached = ctx.uncache(arg["address"])
    if cached:
        return cached
    else:
        a = AmountRange()
        if "amount" in arg:
            a.update_range(amount.convert(arg["amount"]))
        return PayToSegwitAddress(amount=a, address=k)
