from typing import NewType, Union, TYPE_CHECKING
from numpy import uint32, int64, iinfo
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


PubKey = NewType("PubKey", bytes)
Hash = NewType("Hash", bytes)

def Sats(a : int) -> Amount:
    assert a >= 0
    return Amount(int64(a))
def Bitcoin(a : float) -> Amount:
    assert a >= 0
    return Amount(int64(a*100_000_000))

min_int64 = iinfo(int64).min
max_int64 = iinfo(int64).max
max_uint32 = iinfo(uint32).max