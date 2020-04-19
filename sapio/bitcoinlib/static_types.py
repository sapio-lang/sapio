from typing import NewType, Union
from numpy import uint32, int64
Sequence = NewType("Sequence", Union[uint32])
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
