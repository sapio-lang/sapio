use super::*;

fn sapio_v1_wasm_plugin_client_get_create_arguments_nullptr() -> *mut c_char {
    panic!("No Function Registered");
}

unsafe fn sapio_v1_wasm_plugin_client_create_nullptr(_c: *mut c_char) -> *mut c_char {
    panic!("No Function Registered");
}
pub(crate) static mut sapio_v1_wasm_plugin_client_get_create_arguments_ptr: fn() -> *mut c_char =
    sapio_v1_wasm_plugin_client_get_create_arguments_nullptr;

pub(crate) static mut sapio_v1_wasm_plugin_client_create_ptr: unsafe fn(
    *mut c_char,
) -> *mut c_char = sapio_v1_wasm_plugin_client_create_nullptr;

#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_get_create_arguments() -> *mut c_char {
    unsafe { sapio_v1_wasm_plugin_client_get_create_arguments_ptr() }
}

#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_create(c: *mut c_char) -> *mut c_char {
    sapio_v1_wasm_plugin_client_create_ptr(c)
}
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_drop_allocation(s: *mut c_char) {
    CString::from_raw(s);
}
#[no_mangle]
extern "C" fn sapio_v1_wasm_plugin_client_allocate_bytes(len: u32) -> *mut c_char {
    CString::new(vec![1; len as usize]).unwrap().into_raw()
}

pub(crate) static mut sapio_plugin_name: &'static str = "Unnamed";
#[no_mangle]
unsafe extern "C" fn sapio_v1_wasm_plugin_client_get_name() -> *mut c_char {
    CString::new(sapio_plugin_name.as_bytes())
        .unwrap()
        .into_raw()
}
