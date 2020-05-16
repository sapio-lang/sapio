from typing import TypedDict, List, Tuple
from jsonschema import Draft7Validator
from server.context import Context
from server.network.json import address
from bitcoinlib.static_types import Amount
from sapio_compiler import BindableContract

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
        "payments": {
            "type": "array",
            "items": {"type": "object", "properties": {"payment": address.schema}},
        }
    },
    "required": ["payments"],
    "maxProperties": 1,
}
validator = Draft7Validator(schema)


class PayDict(TypedDict):
    payments: List[address.AddrDict]


def convert(arg: PayDict, ctx: Context) -> List[Tuple[Amount, BindableContract]]:
    validator.validate(arg)
    return list(map(lambda p: address.convert(p["payment"], ctx), arg["payments"]))
