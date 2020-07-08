from ctypes import *
import ctypes
import os


lib = ctypes.cdll.LoadLibrary(os.path.dirname(__file__)+"/target/debug/liblibminiscript.so")
lib.make_policy.argtypes = [POINTER(c_char), POINTER(c_size_t), POINTER(POINTER(c_ubyte))]

def compile_policy(key: bytes) -> bytes:
    size = c_size_t(0)
    data =  POINTER(c_ubyte)()
    handle = lib.make_policy(ctypes.c_char_p(b"pk("+key+b")\x00"), byref(size),byref(data))
    ret = bytearray(size.value)
    for i in range(size.value):
        ret[i] = data[i]
    lib.deallocate_policy(handle)
    return bytes(ret)
