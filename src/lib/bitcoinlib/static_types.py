
from ctypes import c_uint32, c_uint8, c_uint16, c_uint64, c_int8, c_int16, c_int32, c_int64
from typing import List, NewType

u8 = NewType("u8", c_uint8)
u16 = NewType("u16", c_uint16)
u32 = NewType("u32", c_uint32)
u64 = NewType("u64", c_uint64)

i8 = NewType("i8", c_int8)
i16 = NewType("i16", c_int16)
i32 = NewType("i32", c_int32)
i64 = NewType("i64", c_int64)

Sequence = NewType("Sequence", u32)
Version = NewType("Version", u32)
LockTime = NewType("LockTime", u32)
Amount = NewType("Amount", i64)


PubKey = NewType("PubKey", bytes)
Hash = NewType("Hash", bytes)
