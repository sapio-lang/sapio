from typing import TypedDict
from jsonschema import Draft7Validator
from server.context import Context

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "number",
    "multipleOf": 1.0,
}
validator = Draft7Validator(schema)


def convert(arg: float) -> int:
    validator.validate(arg)
    return int(arg)
