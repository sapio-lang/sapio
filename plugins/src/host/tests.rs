use super::plugin_handle::SyncModuleLocator;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixtureCache(PathBuf);

impl FixtureCache {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sapio-host-imports-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn load(&self, wat: &str) -> WasmPluginHandle<serde_json::Value> {
        self.try_load(wat).unwrap()
    }

    fn try_load(
        &self,
        wat: &str,
    ) -> Result<WasmPluginHandle<serde_json::Value>, Box<dyn std::error::Error>> {
        WasmPluginHandle::new(
            self.0.clone(),
            SyncModuleLocator::Bytes(wasmer::wat2wasm(wat.as_bytes()).unwrap().into_owned()),
            bitcoin::Network::Regtest,
            None,
        )
    }
}

impl Drop for FixtureCache {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture(data: &[u8], name: &str, api: &str, entry: &str) -> String {
    let encoded = data
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    format!(
        r#"(module
          (import "env" "sapio_v1_wasm_plugin_lookup_module_name"
            (func $lookup (param i32 i32 i32 i32)))
          (import "env" "sapio_v1_wasm_plugin_get_api"
            (func $api (param i32) (result i32)))
          (import "env" "sapio_v1_wasm_plugin_get_logo"
            (func $logo (param i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 8) "ok\00")
          (data (i32.const 1024) "{encoded}")
          (func (export "sapio_v1_wasm_plugin_client_allocate_bytes")
            (param i32) (result i32) i32.const 32768)
          (func (export "sapio_v1_wasm_plugin_client_get_create_arguments")
            (result i32) {api})
          (func (export "sapio_v1_wasm_plugin_client_get_name")
            (result i32) {name})
          (func (export "sapio_v1_wasm_plugin_client_get_logo")
            (result i32) i32.const 8)
          (func (export "sapio_v1_wasm_plugin_client_drop_allocation") (param i32))
          (func (export "sapio_v1_wasm_plugin_client_create")
            (param i32 i32) (result i32) i32.const 8)
          (func (export "sapio_v1_wasm_plugin_entry_point") {entry}))"#
    )
}

#[test]
fn compiler_host_does_not_offer_signing_or_policy_selection_imports() {
    let cache = FixtureCache::new();
    for (name, signature) in [
        (
            "sapio_v1_wasm_plugin_ctv_emulator_sign",
            "(param i32 i32) (result i32)",
        ),
        (
            "sapio_v1_wasm_plugin_ctv_emulator_signer_for",
            "(param i32) (result i32)",
        ),
    ] {
        let source = fixture(&[], "i32.const 8", "i32.const 8", "").replacen(
            "(module",
            &format!("(module (import \"env\" \"{name}\" (func {signature}))"),
            1,
        );
        let error = cache
            .try_load(&source)
            .err()
            .expect("signer import must fail to link");
        assert!(error.to_string().contains(name), "{error}");
    }
}

const FIND_SELF: &str = "i32.const 0 i32.const 0 i32.const 64 i32.const 96 call $lookup";

#[test]
fn guest_allocator_cannot_reenter_an_allocating_host_import() {
    let cache = FixtureCache::new();
    let wat = fixture(
        &[],
        &format!("{FIND_SELF} i32.const 64 call $logo drop i32.const 8"),
        "i32.const 8",
        "",
    )
    .replacen(
        "(memory (export \"memory\") 1)",
        "(global $allocations (mut i32) (i32.const 0)) (memory (export \"memory\") 1)",
        1,
    )
    .replacen(
        "(param i32) (result i32) i32.const 32768)",
        "(param i32) (result i32)
         global.get $allocations i32.const 1 i32.add global.set $allocations
         global.get $allocations i32.const 16 i32.lt_u
         if i32.const 64 call $logo drop end
         i32.const 32768)",
        1,
    )
    .replacen(
        "(export \"sapio_v1_wasm_plugin_client_get_logo\")\n            (result i32) i32.const 8",
        "(export \"sapio_v1_wasm_plugin_client_get_logo\")\n            (result i32) i32.const 512 global.get $allocations i32.const 48 i32.add i32.store8 i32.const 512",
        1,
    );
    let mut plugin = cache.load(&wat);
    // The adversarial recursion is finite even without the guard. The guest
    // counter also proves a failed callback releases the guard for the next call.
    for count in ["1", "2"] {
        let error = plugin.get_name().unwrap_err();
        assert!(error.to_string().contains("allocator reentry"), "{error}");
        assert_eq!(plugin.get_logo().unwrap(), count);
    }
}

#[test]
fn recursive_guest_api_stops_at_the_depth_limit() {
    let cache = FixtureCache::new();
    let wat = fixture(
        &[],
        &format!("{FIND_SELF} i32.const 64 call $logo drop i32.const 8"),
        &format!("{FIND_SELF} i32.const 64 call $api"),
        "",
    );
    let mut plugin = cache.load(&wat);
    assert!(plugin.get_api().is_err());
    // The recursion consumes exactly MAX_DEPTH successful children and the
    // rejected next attempt. Exercise the remaining allowance through real calls.
    for _ in 0..invocation::MAX_MODULE_CALLS - invocation::MAX_DEPTH - 1 {
        assert_eq!(plugin.get_name().unwrap(), "ok");
    }
    assert!(plugin
        .get_name()
        .unwrap_err()
        .to_string()
        .contains("nested module call limit"));
}

fn sibling_calls(count: usize, find_key: &str) -> String {
    format!(
        "(local $calls i32) {find_key}
         (loop $again
           i32.const 64 call $logo drop
           local.get $calls i32.const 1 i32.add local.tee $calls
           i32.const {count} i32.lt_u br_if $again)
         i32.const 8"
    )
}

#[test]
fn sibling_guest_calls_share_a_nonrenewable_allowance_and_fresh_clone_resets_it() {
    let cache = FixtureCache::new();
    let wat = fixture(
        &[],
        &sibling_calls(invocation::MAX_MODULE_CALLS, FIND_SELF),
        "i32.const 8",
        "",
    );
    let mut plugin = cache.load(&wat);
    assert_eq!(plugin.get_name().unwrap(), "ok");
    let error = plugin.get_name().unwrap_err();
    assert!(
        error.to_string().contains("nested module call limit"),
        "{error}"
    );
    let mut fresh = plugin.fresh_clone().unwrap();
    assert_eq!(fresh.get_name().unwrap(), "ok");
    assert!(fresh
        .get_name()
        .unwrap_err()
        .to_string()
        .contains("nested module call limit"));
}

#[test]
fn failed_child_lookups_consume_the_guest_call_allowance() {
    let cache = FixtureCache::new();
    // The zero hash at offset 64 names no cached module. The guest ignores
    // these ordinary lookup errors, but cannot keep issuing calls forever.
    let wat = fixture(
        &[],
        &sibling_calls(invocation::MAX_MODULE_CALLS, ""),
        "i32.const 8",
        "",
    );
    let mut plugin = cache.load(&wat);
    assert_eq!(plugin.get_name().unwrap(), "ok");
    let error = plugin.get_name().unwrap_err();
    assert!(
        error.to_string().contains("nested module call limit"),
        "{error}"
    );
}
