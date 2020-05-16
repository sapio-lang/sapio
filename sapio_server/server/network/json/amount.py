from typing import TypedDict
from typing import Union
from jsonschema import Draft7Validator
from server.context import Context
from bitcoinlib.static_types import Amount

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "oneOf": [
        {
            "title": "Bitcoin",
            "properties": {"btc": {"type": "number", "min": 0, "max": 21e6}},
            "required": ["btc"],
            "maxProperties": 1,
        },
        {
            "title": "Sats",
            "properties": {
                "sats": {"type": "number", "min": 0, "max": 21_000_000 * 100_000_000}
            },
            "required": ["sats"],
            "maxProperties": 1,
        },
    ],
}
validator = Draft7Validator(schema)


class Bitcoin(TypedDict):
    btc: float


class Satoshis(TypedDict):
    sats: int


def convert(arg: Union[Bitcoin, Satoshis], ctx: Context) -> Amount:
    validator.validate(arg)
    if "btc" in arg:
        a = arg["btc"]
        return Amount(int64(a * 100_000_000))
    if "sats" in arg:
        s = arg["sats"]
        return Amount(int64(s))
