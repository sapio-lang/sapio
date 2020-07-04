from typing import TypedDict
from jsonschema import Draft7Validator
from sapio_server.context import Context
from sapio_bitcoinlib.static_types import PubKey

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {"pubkey": {"type": "string"}},
    "required": ["pubkey"],
    "maxProperties": 1,
}
validator = Draft7Validator(schema)


class PKDict(TypedDict):
    pubkey: str


def convert(arg: PKDict, ctx: Context) -> PubKey:
    validator.validate(arg)
    return PubKey(arg["pubkey"])
