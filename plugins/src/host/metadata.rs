//! Disposable, content-addressed discovery metadata. Execution always obtains
//! its validation schemas from the running module, never this cache.

use super::plugin_handle::{ModuleLocator, SyncModuleLocator};
use super::{wasm_cache, PluginHandle, WasmPluginHandle};
use crate::API;
use bitcoin::Network;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use wasmer_cache::Hash;

const FORMAT_VERSION: u32 = 1;
const MAX_METADATA_BYTES: u64 = 48 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Public information used for module discovery and editors.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleMetadata {
    /// Exact source identifier.
    pub key: String,
    /// Module-provided display name.
    pub name: String,
    /// Module-provided schemas, validated offline before publication.
    pub api: API<crate::CreateArgs<Value>, Value>,
    /// Module-provided image data URL.
    pub logo: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cached {
    version: u32,
    checksum: String,
    metadata: ModuleMetadata,
}

fn cache_path(path: &Path, key: Hash) -> PathBuf {
    path.join("metadata").join(format!("{key}.json"))
}

fn validate(metadata: &ModuleMetadata, key: Hash) -> Result<(), Box<dyn Error>> {
    if metadata.key != key.to_string() {
        return Err("Metadata source key mismatch".into());
    }
    super::validation::CallSchema::from_json(&serde_json::to_vec(&metadata.api)?)?;
    Ok(())
}

fn read_cached(path: &Path, key: Hash) -> Option<ModuleMetadata> {
    let path = cache_path(path, key);
    if !fs::symlink_metadata(&path).ok()?.file_type().is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_METADATA_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return None;
    }
    let cached: Cached = serde_json::from_slice(&bytes).ok()?;
    if cached.version != FORMAT_VERSION
        || cached.checksum
            != Hash::generate(&serde_json::to_vec(&cached.metadata).ok()?).to_string()
        || validate(&cached.metadata, key).is_err()
    {
        return None;
    }
    Some(cached.metadata)
}

/// Capture and cache metadata from an already loaded module.
pub fn capture(
    path: &Path,
    plugin: &mut WasmPluginHandle<Value>,
) -> Result<ModuleMetadata, Box<dyn Error>> {
    let key = plugin.id();
    let metadata = ModuleMetadata {
        key: key.to_string(),
        name: plugin.get_name()?,
        api: plugin.get_api()?,
        logo: plugin.get_logo()?,
    };
    validate(&metadata, key)?;
    let cached = Cached {
        version: FORMAT_VERSION,
        checksum: Hash::generate(&serde_json::to_vec(&metadata)?).to_string(),
        metadata,
    };
    let bytes = serde_json::to_vec(&cached)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err("Module metadata exceeds 48 MiB".into());
    }
    let destination = cache_path(path, key);
    let directory = destination.parent().expect("metadata directory");
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(
        ".{key}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        drop(file);
        fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result?;
    Ok(cached.metadata)
}

/// Read discovery metadata without compiling or instantiating warm entries.
/// The source bytes are authenticated even when the metadata is cached. Missing,
/// stale, or corrupt metadata is reconstructed from a normally metered module.
pub fn get(
    path: &Path,
    locator: SyncModuleLocator,
    net: Network,
    plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
) -> Result<ModuleMetadata, Box<dyn Error>> {
    get_or_derive(path, locator, |locator| {
        let mut plugin = WasmPluginHandle::new(path, locator, net, plugin_map)?;
        capture(path, &mut plugin)
    })
}

fn get_or_derive(
    path: &Path,
    locator: SyncModuleLocator,
    derive: impl FnOnce(SyncModuleLocator) -> Result<ModuleMetadata, Box<dyn Error>>,
) -> Result<ModuleMetadata, Box<dyn Error>> {
    if let SyncModuleLocator::Key(key) = &locator {
        wasm_cache::read_source(&wasm_cache::source_path(path, *key), *key)?;
        if let Some(metadata) = read_cached(path, *key) {
            return Ok(metadata);
        }
    }
    derive(locator)
}

/// Resolve a file or source key and read its discovery metadata.
pub async fn get_async(
    path: &Path,
    locator: ModuleLocator,
    net: Network,
    plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
) -> Result<ModuleMetadata, Box<dyn Error>> {
    get(path, locator.locate().await?, net, plugin_map)
}

/// List cached modules in key order, compiling only entries without metadata.
pub fn list(
    path: &Path,
    net: Network,
    plugin_map: Option<BTreeMap<Vec<u8>, [u8; 32]>>,
) -> Result<Vec<ModuleMetadata>, Box<dyn Error>> {
    wasm_cache::get_all_keys_from_fs(path)?
        .into_iter()
        .map(|key| {
            get(
                path,
                SyncModuleLocator::Key(Hash::from_str(&key)?),
                net,
                plugin_map.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_discovery_skips_execution_and_corrupt_metadata_is_rederived() {
        let path = std::env::temp_dir().join(format!(
            "sapio-discovery-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = wasmer::wat2wasm(br#"(module
          (memory (export "memory") 1)
          (data (i32.const 8) "demo\00")
          (data (i32.const 64) "{\22arguments\22:{},\22returns\22:{}}\00")
          (func (export "sapio_v1_wasm_plugin_client_allocate_bytes") (param i32) (result i32) i32.const 4096)
          (func (export "sapio_v1_wasm_plugin_client_drop_allocation") (param i32))
          (func (export "sapio_v1_wasm_plugin_client_get_create_arguments") (result i32) i32.const 64)
          (func (export "sapio_v1_wasm_plugin_client_get_name") (result i32) i32.const 8)
          (func (export "sapio_v1_wasm_plugin_client_get_logo") (result i32) i32.const 8)
          (func (export "sapio_v1_wasm_plugin_client_create") (param i32 i32) (result i32) unreachable)
          (func (export "sapio_v1_wasm_plugin_entry_point")))"#).unwrap().into_owned();
        let key = Hash::generate(&bytes);
        assert_eq!(
            get(
                &path,
                SyncModuleLocator::Bytes(bytes),
                Network::Regtest,
                None
            )
            .unwrap()
            .name,
            "demo"
        );
        assert_eq!(
            get_or_derive(&path, SyncModuleLocator::Key(key), |_| panic!(
                "warm metadata instantiated WASM"
            ))
            .unwrap()
            .name,
            "demo"
        );
        let original = fs::read(cache_path(&path, key)).unwrap();
        for corrupt in [
            b"truncated".to_vec(),
            {
                let mut value: Value = serde_json::from_slice(&original).unwrap();
                value["version"] = 0.into();
                serde_json::to_vec(&value).unwrap()
            },
            {
                let mut value: Value = serde_json::from_slice(&original).unwrap();
                value["metadata"]["name"] = "changed".into();
                serde_json::to_vec(&value).unwrap()
            },
        ] {
            fs::write(cache_path(&path, key), corrupt).unwrap();
            assert_eq!(
                get(&path, SyncModuleLocator::Key(key), Network::Regtest, None)
                    .unwrap()
                    .name,
                "demo"
            );
        }
        fs::write(wasm_cache::source_path(&path, key), b"bad source").unwrap();
        assert!(
            get_or_derive(&path, SyncModuleLocator::Key(key), |_| panic!(
                "source must authenticate first"
            ))
            .is_err()
        );
        fs::remove_dir_all(path).unwrap();
    }
}
