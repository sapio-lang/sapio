from typing import TypedDict, Union

from jsonschema import Draft7Validator
from sapio_server.context import Context
from sapio_server.network.json import amount
from sapio_compiler import BindableContract, AmountRange
from sapio_zoo.p2pk import PayToSegwitAddress

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
        "address": {
            "type": "string",
            # https://stackoverflow.com/a/59756959/865714
            # just used as a basic sanity check...
            "pattern": r"\b(bc(0([ac-hj-np-z02-9]{39}|[ac-hj-np-z02-9]{59})|1[ac-hj-np-z02-9]{8,87})|[13][a-km-zA-HJ-NP-Z1-9]{25,35})\b",
        },
        "amount": amount.schema,
    },
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
            a.update_range(amount.convert(arg["amount"], ctx))
        return PayToSegwitAddress(amount=a, address=arg["address"])
