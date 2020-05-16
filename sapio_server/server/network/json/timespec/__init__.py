from typing import Union
from jsonschema import Draft7Validator
from server.context import Context
from server.network.json.timespec import absolute
from server.network.json.timespec import relative
from sapio_compiler import RelativeTimeSpec, AbsoluteTimeSpec

schema = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "oneOf": [absolute.schema, relative.schema],
}

validator = Draft7Validator(schema)


def convert(
    arg: Union[absolute.AbsoluteDict, relative.RelativeDict], ctx: Context
) -> Union[RelativeTimeSpec, AbsoluteTimeSpec]:
    validator.validate(arg)
    try:
        return absolute.convert(arg)
    except:
        return relative.convert(arg)


del Context
del Draft7Validator
