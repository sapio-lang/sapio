from typing import TypedDict, Union

from jsonschema import Draft7Validator
from server.context import Context
from bitcoin_script_compiler import RelativeTimeSpec

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "oneOf": [
        {
            "title": "Relative Days",
            "properties": {
                "days": {
                    "type": "number",
                    "min": 0,
                    "max": 0x0FFFF * 512 / 60 / 60 / 24,
                }
            },
            "required": ["days"],
            "maxProperties": 1,
        },
        {
            "title": "Relative Weeks",
            "properties": {
                "weeks": {
                    "type": "number",
                    "min": 0,
                    "max": 0x0FFFF * 512 / 60 / 60 / 24 / 7,
                }
            },
            "required": ["weeks"],
            "maxProperties": 1,
        },
        {
            "title": "Relative Blocks",
            "properties": {"blocks": {"type": "number", "min": 0, "max": 0x0FFFF}},
            "required": ["blocks"],
            "maxProperties": 1,
        },
    ],
}


class Days(TypedDict):
    days: float


class Weeks(TypedDict):
    weeks: float


class Blocks(TypedDict):
    blocks: int


validator = Draft7Validator(schema)

RelativeDict = Union[Days, Weeks, Blocks]


def convert(arg: Union[Days, Weeks, Blocks], ctx: Context) -> RelativeTimeSpec:
    validator.validate(arg)
    if "days" in arg:
        days = arg["days"]
        return Days(days)
    if "weeks" in arg:
        weeks = arg["days"]
        return Weeks(weeks)
    if "blocks" in arg:
        blocks = arg["blocks"]
        return RelativeTimeSpec.blocks_later(blocks)


def convert_sequence(arg: Union[Days, Weeks, Blocks], ctx: Context) -> RelativeTimeSpec:
    return convert(arg, ctx).sequence
