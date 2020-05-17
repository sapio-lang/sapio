from typing import NewType, TYPE_CHECKING
from numpy import uint32, int64, iinfo
import bitcoinlib

if TYPE_CHECKING:
    Sequence = NewType("Sequence", int)
    Version = NewType("Version", int)
    LockTime = NewType("LockTime", int)
    Amount = NewType("Amount", int)
else:
    Sequence = NewType("Sequence", uint32)
    Version = NewType("Version", uint32)
    LockTime = NewType("LockTime", uint32)
    Amount = NewType("Amount", int64)



class PubKey(bytes):
    def __new__(self, b):
        import bitcoinlib.address
        try:
            return super().__new__(bitcoinlib.address.check_key(b))
        except:
            raise ValueError("Not a Valid key", b)


Hash = NewType("Hash", bytes)


def Sats(a: int) -> Amount:
    assert a >= 0
    return Amount(int64(a))


def Bitcoin(a: float) -> Amount:
    assert a >= 0
    return Amount(int64(a * 100_000_000))


min_int64 = iinfo(int64).min
max_int64 = iinfo(int64).max
max_uint32 = iinfo(uint32).max
