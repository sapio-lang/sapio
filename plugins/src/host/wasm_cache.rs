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
use std::ffi::OsString;
pub fn get_all_keys_from_fs(
    typ: &str,
    org: &str,
    proj: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    std::fs::read_dir(get_path(typ, org, proj).into())?
        .map(|entry| {
            match entry.map(|x| {
                x.path()
                    .file_stem()
                    .map(|f| f.to_str().map(String::from))
                    .flatten()
                    .ok_or(String::from("Nothing").into())
            }) {
                Ok(x) => x,
                Err(x) => Err(x.into()),
            }
        })
        .collect()
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
