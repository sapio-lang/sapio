from typing import Dict, Type, Callable, Any, Union, Tuple, Optional, List, TypedDict


from sapio_zoo.p2pk import PayToSegwitAddress
from bitcoinlib.static_types import Amount, Sequence, PubKey
from bitcoinlib.static_types import int64

from sapio_compiler import (
    Contract,
    RelativeTimeSpec,
    AbsoluteTimeSpec,
    AmountRange,
    BindableContract,
    Days,
    Weeks,
)
from .context import Context


import jsonschema

import server.network.json as schemas

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
    int: schemas.int.schema,
    List[Tuple[Amount, Contract]]: schemas.payments.schema,
}


def create_jsonschema(hints: Dict[str, Type]):
    return {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": {s: dict(**subschemas[t], **{"title": s}) for (s, t) in hints},
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
    int: schemas.int.convert,
    str: lambda x, y: x,
    Union[AbsoluteTimeSpec, RelativeTimeSpec]: schemas.timespec.convert,
    PayToSegwitAddress: schemas.address.convert,
}
