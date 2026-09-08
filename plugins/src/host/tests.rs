use super::plugin_handle::SyncModuleLocator;
use super::*;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::util::psbt::raw;
use bitcoin::{EcdsaSig, EcdsaSighashType, PublicKey, Script, Transaction, TxIn, TxOut};
use sapio_ctv_emulator_trait::{Clause, EmulatorError};
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

    fn load(
        &self,
        wat: &str,
        emulator: &Arc<dyn CTVEmulator>,
    ) -> WasmPluginHandle<serde_json::Value> {
        WasmPluginHandle::new(
            self.0.clone(),
            emulator,
            SyncModuleLocator::Bytes(wasmer::wat2wasm(wat.as_bytes()).unwrap().into_owned()),
            bitcoin::Network::Regtest,
            None,
        )
        .unwrap()
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
          (import "env" "sapio_v1_wasm_plugin_ctv_emulator_sign"
            (func $sign (param i32 i32) (result i32)))
          (import "env" "sapio_v1_wasm_plugin_ctv_emulator_signer_for"
            (func $signer_for (param i32) (result i32)))
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

fn signing_request() -> PartiallySignedTransaction {
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: 9000,
            script_pubkey: Script::from(vec![0x51]),
        }],
    })
    .unwrap();
    psbt.unknown.insert(
        raw::Key {
            type_value: 0x80,
            key: vec![1],
        },
        vec![2, 3],
    );
    psbt
}

struct SigningEmulator(fn(&mut PartiallySignedTransaction));

impl CTVEmulator for SigningEmulator {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::TxTemplate(h))
    }

    fn sign(
        &self,
        mut psbt: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        self.0(&mut psbt);
        Ok(psbt)
    }
}

#[test]
fn signing_import_rejects_transaction_and_metadata_changes() {
    let cache = FixtureCache::new();
    let bytes = serde_json::to_vec(&signing_request()).unwrap();
    let wat = fixture(
        &bytes,
        &format!("i32.const 1024 i32.const {} call $sign", bytes.len()),
        "i32.const 8",
        "",
    );
    for mutate in [
        (|psbt: &mut PartiallySignedTransaction| psbt.unsigned_tx.output[0].value += 1)
            as fn(&mut PartiallySignedTransaction),
        |psbt: &mut PartiallySignedTransaction| psbt.unknown.clear(),
    ] {
        let emulator: Arc<dyn CTVEmulator> = Arc::new(SigningEmulator(mutate));
        let mut plugin = cache.load(&wat, &emulator);
        let error = plugin
            .get_name()
            .expect_err("the host must reject a signer that changes the PSBT");
        assert!(
            error
                .to_string()
                .contains("Emulator response must preserve the PSBT and only add signatures"),
            "{error}"
        );
    }
}

#[test]
fn signing_import_preserves_psbt_and_accepts_signature_additions() {
    fn add_signature(psbt: &mut PartiallySignedTransaction) {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[7; 32]).unwrap();
        let public = PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(
            &secp, &secret,
        ));
        let message = Message::from_digest_slice(&[9; 32]).unwrap();
        psbt.inputs[0].partial_sigs.insert(
            public,
            EcdsaSig {
                sig: secp.sign_ecdsa(&message, &secret),
                hash_ty: EcdsaSighashType::All,
            },
        );
    }
    let cache = FixtureCache::new();
    let mut expected = signing_request();
    let bytes = serde_json::to_vec(&expected).unwrap();
    let emulator: Arc<dyn CTVEmulator> = Arc::new(SigningEmulator(add_signature));
    let wat = fixture(
        &bytes,
        &format!("i32.const 1024 i32.const {} call $sign", bytes.len()),
        "i32.const 8",
        "",
    );
    let mut plugin = cache.load(&wat, &emulator);
    let response: PartiallySignedTransaction =
        serde_json::from_str(&plugin.get_name().unwrap()).unwrap();
    add_signature(&mut expected);
    assert_eq!(response, expected);
}

struct CountingEmulator(AtomicUsize);

impl CTVEmulator for CountingEmulator {
    fn get_signer_for(&self, h: sha256::Hash) -> Result<Clause, EmulatorError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Clause::TxTemplate(h))
    }

    fn sign(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        Ok(psbt)
    }
}

const FIND_SELF: &str = "i32.const 0 i32.const 0 i32.const 64 i32.const 96 call $lookup";
const COUNT_INSTANCE: &str = "i32.const 128 call $signer_for drop";

#[test]
fn guest_allocator_cannot_reenter_an_allocating_host_import() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CountingEmulator(AtomicUsize::new(0)));
    let emulator: Arc<dyn CTVEmulator> = counter.clone();
    let wat = fixture(&[], "i32.const 128 call $signer_for", "i32.const 8", "")
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
             if i32.const 128 call $signer_for drop end
             i32.const 32768)",
            1,
        );
    let mut plugin = cache.load(&wat, &emulator);
    // The adversarial recursion is finite even without the guard; this test
    // cannot overflow the native stack when demonstrating the old behavior.
    let error = plugin.get_name().unwrap_err();
    assert!(error.to_string().contains("allocator reentry"), "{error}");
    assert_eq!(counter.0.load(Ordering::Relaxed), 2);
    // A failed allocator callback must release the guard before the next call.
    let error = plugin.get_name().unwrap_err();
    assert!(error.to_string().contains("allocator reentry"), "{error}");
    assert_eq!(counter.0.load(Ordering::Relaxed), 4);
}

#[test]
fn recursive_guest_api_stops_at_the_depth_limit() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CountingEmulator(AtomicUsize::new(0)));
    let emulator: Arc<dyn CTVEmulator> = counter.clone();
    let wat = fixture(
        &[],
        "i32.const 8",
        &format!("{FIND_SELF} i32.const 64 call $api"),
        COUNT_INSTANCE,
    );
    let mut plugin = cache.load(&wat, &emulator);
    assert!(plugin.get_api().is_err());
    // Count initialization through a real host import, including the root.
    assert_eq!(counter.0.load(Ordering::Relaxed), invocation::MAX_DEPTH + 1);
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
    let counter = Arc::new(CountingEmulator(AtomicUsize::new(0)));
    let emulator: Arc<dyn CTVEmulator> = counter.clone();
    let wat = fixture(
        &[],
        &sibling_calls(invocation::MAX_MODULE_CALLS, FIND_SELF),
        "i32.const 8",
        COUNT_INSTANCE,
    );
    let mut plugin = cache.load(&wat, &emulator);
    assert_eq!(plugin.get_name().unwrap(), "ok");
    assert_eq!(
        counter.0.load(Ordering::Relaxed),
        invocation::MAX_MODULE_CALLS + 1
    );
    let error = plugin.get_name().unwrap_err();
    assert!(
        error.to_string().contains("nested module call limit"),
        "{error}"
    );
    assert_eq!(
        counter.0.load(Ordering::Relaxed),
        invocation::MAX_MODULE_CALLS + 1
    );

    let mut fresh = plugin.fresh_clone().unwrap();
    assert_eq!(fresh.get_name().unwrap(), "ok");
    assert_eq!(
        counter.0.load(Ordering::Relaxed),
        2 * (invocation::MAX_MODULE_CALLS + 1)
    );
    assert!(fresh
        .get_name()
        .unwrap_err()
        .to_string()
        .contains("nested module call limit"));
}

#[test]
fn failed_child_lookups_consume_the_guest_call_allowance() {
    let cache = FixtureCache::new();
    let counter = Arc::new(CountingEmulator(AtomicUsize::new(0)));
    let emulator: Arc<dyn CTVEmulator> = counter.clone();
    // The zero hash at offset 64 names no cached module. The guest ignores
    // these ordinary lookup errors, but cannot keep issuing calls forever.
    let wat = fixture(
        &[],
        &sibling_calls(invocation::MAX_MODULE_CALLS, ""),
        "i32.const 8",
        COUNT_INSTANCE,
    );
    let mut plugin = cache.load(&wat, &emulator);
    assert_eq!(plugin.get_name().unwrap(), "ok");
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    let error = plugin.get_name().unwrap_err();
    assert!(
        error.to_string().contains("nested module call limit"),
        "{error}"
    );
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);
}
