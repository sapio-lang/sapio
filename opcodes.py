def b(i):
    return    bytes([i])


import bitcoinlib
from bitcoinlib.script import *


# TODO: Phase these out or re-think this as required ops subset
class Op:
    And = OP_AND
    Or = OP_OR
    Not = OP_NOT
    Check_sig_verify = OP_CHECKSIGVERIFY
    Sha256 = OP_SHA256
    Equal = OP_EQUAL
    Drop = OP_DROP
    Pick = OP_PICK
    Depth = OP_DEPTH
    Sub = OP_SUB
    Drop2 = OP_2DROP
    SubOne = OP_1SUB
    IfDup = OP_IFDUP
    NotIf = OP_NOTIF
    EndIf = OP_ENDIF
    Zero = OP_0
    CheckTemplateVerify = OP_CHECKTEMPLATEVERIFY
    CheckLockTimeVerify = OP_CHECKLOCKTIMEVERIFY
    CheckSequenceVerify = OP_CHECKSEQUENCEVERIFY
    Dup = OP_DUP
    Within = OP_WITHIN
    Verify = OP_VERIFY
    If = OP_IF
    Else = OP_ELSE


# TODO: Make real
def PushData(data):
    if isinstance(data, bytes):
        op = Op.PushByte + bytes([len(data)])
        return b"".join([op, data])
    if isinstance(data, int):
        return b"".join(bytes([Op.PushByte, data]), data)


def PushNumber(value):
    if value <= 16:
        return b" "+b(value)
    else:
        r = bytearray(0)
        if value == 0:
            return bytes(r)
        neg = value < 0
        absvalue = -value if neg else value
        while (absvalue):
            r.append(absvalue & 0xff)
            absvalue >>= 8
        if r[-1] & 0x80:
            r.append(0x80 if neg else 0)
        elif neg:
            r[-1] |= 0x80
        return bytes([len(r)]) + r


