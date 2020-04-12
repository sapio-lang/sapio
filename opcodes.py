def b(i):
    return    bytes([i])
# Todo: copy out of script/interpreter
class Op:
    And = b(1)
    Or = b(2)
    Not = b(3)
    Check_sig_verify = b(4)
    Sha256 = b(5)
    Equal = b(6)
    Drop = b(7)
    Pick = b(8)
    Depth = b(9)
    PushByte = b(10)
    Sub = b(11)
    Drop2 = b(12)

class Op:
    And = b" AND"
    Or = b" OR"
    Not = b" NOT"
    Check_sig_verify = b" CHECKSIGVERIFY"
    Sha256 = b" SHA256"
    Equal = b" EQUAL"
    Drop = b" DROP"
    Pick = b" PICK"
    Depth = b" DEPTH"
    PushByte = b" PUSHBYTE"
    Sub = b" SUB"
    Drop2 = b" DROP2"
    SubOne = b" SubOne"
    IfDup = b" IfDup"
    NotIf = b" NotIf"
    EndIf = b" EndIf"
    Zero = b" Zero"
    CheckTemplateVerify = b" CheckTemplateVerify"
    CheckLockTimeVerify = b" CheckLockTimeVerify"
    CheckSequenceVerify = b" CheckSequenceVerify"
    Dup = b" Dup"
    Within = b" Within"
    Verify = b" Verify"
    If = b" If"
    Else = b" Else"


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


