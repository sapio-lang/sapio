use std::path::PathBuf;
use wasmer::{DeserializeError, Module, SerializeError, Store};
use wasmer_cache::{Cache, FileSystemCache, Hash};

fn get_path(typ: &str, org: &str, proj: &str) -> impl Into<PathBuf> {
    let proj =
        directories::ProjectDirs::from(typ, org, proj).expect("Failed to find config directory");
    let mut path: PathBuf = proj.data_dir().clone().into();
    path.push("modules");
    path
}

pub fn load_module(
    typ: &str,
    org: &str,
    proj: &str,
    store: &Store,
    bytes: &[u8],
) -> Result<(Module, Hash), DeserializeError> {
    let path = get_path(typ, org, proj);
    let key = Hash::generate(bytes);
    let f = FileSystemCache::new(path)?;
    unsafe { f.load(store, key) }.map(|m| (m, key))
}

pub fn load_module_key(
    typ: &str,
    org: &str,
    proj: &str,
    store: &Store,
    key: Hash,
) -> Result<(Module, Hash), DeserializeError> {
    let path = get_path(typ, org, proj);
    let f = FileSystemCache::new(path)?;
    unsafe { f.load(store, key) }.map(|m| (m, key))
}
pub fn store_module(
    typ: &str,
    org: &str,
    proj: &str,
    module: &Module,
    bytes: &[u8],
) -> Result<Hash, SerializeError> {
    let path = get_path(typ, org, proj);
    let mut cache = FileSystemCache::new(path)?;
    #[cfg(target_os = "windows")]
    {
        cache.set_cache_extension(Some("dll"))
    }
    let key = Hash::generate(bytes);

    cache.store(key, module)?;
    Ok(key)
}
