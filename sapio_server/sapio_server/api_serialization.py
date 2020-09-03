from typing import Callable, Dict, List, Tuple, Type, Union


from sapio_zoo.p2pk import PayToSegwitAddress
from sapio_bitcoinlib.static_types import Amount, Sequence, PubKey
from sapio_bitcoinlib.static_types import int64

from sapio_compiler import (
    RelativeTimeSpec,
    AbsoluteTimeSpec,
    AmountRange,
    Contract,
    Days,
    Weeks,
)
from .context import Context


import jsonschema

import sapio_server.network.json as schemas

print(dir(schemas))
subschemas = {
    Amount: schemas.amount.schema,
    Sequence: schemas.timespec.relative.schema,
    RelativeTimeSpec: schemas.timespec.relative.schema,
    AbsoluteTimeSpec: schemas.timespec.absolute.schema,
    Union[RelativeTimeSpec, AbsoluteTimeSpec]: schemas.timespec.schema,
    PayToSegwitAddress: schemas.address.schema,
    Contract: schemas.address.schema,
    PubKey: schemas.pubkey.schema,
    int: schemas.integer.schema,
    List[Tuple[Amount, Contract]]: schemas.payments.schema,
}


def create_jsonschema(name: str, hints: Dict[str, Type]):
    return {
        "title": name,
        "properties": dict(**{s: subschemas[t] for (s, t) in hints}),
        "required": [s for (s, _) in hints],
    }


conversion_functions: Dict[Type, Callable]
"""
conversion_functions is hand declared so that the type lookup registers
properly for newtyped declarations
"""

conversion_functions = {
    PubKey: schemas.pubkey.convert,
    Contract: schemas.address.convert,
    List[Tuple[Amount, Contract]]: schemas.payments.convert,
    Tuple[Amount, Contract]: schemas.address.convert,
    Amount: schemas.amount.convert,
    Sequence: schemas.timespec.relative.convert_sequence,
    RelativeTimeSpec: schemas.timespec.relative.convert,
    int: schemas.integer.convert,
    str: lambda x, y: x,
    Union[AbsoluteTimeSpec, RelativeTimeSpec]: schemas.timespec.convert,
    PayToSegwitAddress: schemas.address.convert,
}
