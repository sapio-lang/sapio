from typing import TypedDict, Literal
from typing import Union
from jsonschema import Draft7Validator
from sapio_server.context import Context
from bitcoinlib.static_types import Amount, int64

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "description": "An amount of coins.",
    "properties": {"units": {"enum": ["Bitcoin", "Sats"], "default": "Bitcoin"},},
    "required": ["units", "amount"],
    "dependencies": {
        "units": {
            "oneOf": [
                {
                    "properties": {
                        "units": {"enum": ["Bitcoin"]},
                        "amount": {"type": "number", "min": 0, "max": 21e6,},
                    },
                },
                {
                    "properties": {
                        "units": {"enum": ["Sats"]},
                        "amount": {
                            "type": "integer",
                            "min": 0,
                            "max": 21_000_000 * 100_000_000,
                            "multipleOf": 1.0,
                        },
                    },
                },
            ]
        }
    },
}
validator = Draft7Validator(schema)


class Units(TypedDict):
    units: Literal["Bitcoin", "Sats"]


class Bitcoin(Units):
    amount: float


class Satoshis(Units):
    amount: int


def convert(arg: Union[Bitcoin, Satoshis], ctx: Context) -> Amount:
    validator.validate(arg)
    a = arg["amount"]
    if arg["units"] == "Bitcoin":
        return Amount(int64(a * 100_000_000))
    if arg["units"] == "Sats":
        return Amount(int64(a))
