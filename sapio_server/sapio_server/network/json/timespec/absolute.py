from typing import Union
from jsonschema import Draft7Validator
from sapio_server.context import Context
from bitcoin_script_compiler import AbsoluteTimeSpec
import datetime

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "oneOf": [
        {"title": "Block Height", "type": "number", "min": 0, "multipleOf": 1.0,},
        {"title": "Network Date", "type": "string", "format": "date-time",},
    ],
}


Height = Union[int, float]
Date = str
AbsoluteDict = Union[Date, Height]
validator = Draft7Validator(schema)


def convert(arg: AbsoluteDict, ctx: Context) -> AbsoluteTimeSpec:
    validator.validate(arg)
    if isinstance(arg, str):
        AbsoluteTimeSpec.from_date(datetime.fromisoformat(arg))
    else:
        AbsoluteTimeSpec.at_height(int(arg))
