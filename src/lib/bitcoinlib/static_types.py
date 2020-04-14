
from typing import List, NewType
from numpy import uint8, uint16, uint32, uint64, int8, int16, int32, int64

uint32 : uint32
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
