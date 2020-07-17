import os
from cffi import FFI

ffi = FFI()
ffi.cdef(
    """
         const void* make_policy(const char* input, size_t* len, const uint8_t * const * out);
         """
)
ffi.cdef(
    """
          void deallocate_policy(const void* policy);
         """
)


libname = "actual_lib"
lib = None
path = os.path.dirname(__file__)
for f in os.listdir(path):
    if f[: len(libname)] == libname:
        lib = ffi.dlopen(path + "/" + f)


def compile_policy(key: bytes) -> bytes:
    size = ffi.new("size_t *")
    data = ffi.new("const uint8_t const **")
    handle = lib.make_policy(key, size, data)
    if handle == ffi.NULL or size[0] == ffi.NULL or data[0] == 0:
        raise ValueError(f"Bad Policy! {key}")
    ret = bytearray(size[0])
    for i in range(size[0]):
        ret[i] = data[0][i]
    lib.deallocate_policy(handle)
    return ret
