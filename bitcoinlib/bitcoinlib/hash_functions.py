import hashlib
from typing import Union
AnyBytes = Union[bytes, bytearray]

def sha256(s: AnyBytes) -> bytes:
    return hashlib.new('sha256', s).digest()


def hash256(s: AnyBytes)->bytes:
    return sha256(sha256(s))
