// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! tools for caching compilations of wasm plugins to disk
use std::path::PathBuf;
use wasmer::{DeserializeError, Module, SerializeError, Store};
use wasmer_cache::{Cache, FileSystemCache, Hash};

/// look at the cache and get all of the keys (as Strings) for plugins
pub fn get_all_keys_from_fs<I: Into<PathBuf>>(
    path: I,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    std::fs::read_dir(path.into())?
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

/// load a module given the bytes of the module, may consult cache if available
pub fn load_module<I: Into<PathBuf>>(
    path: I,
    store: &Store,
    bytes: &[u8],
) -> Result<(Module, Hash), DeserializeError> {
    let key = Hash::generate(bytes);
    let f = FileSystemCache::new(path)?;
    unsafe { f.load(store, key) }.map(|m| (m, key))
}

/// load a module from the cache
pub fn load_module_key<I: Into<PathBuf>>(
    path: I,
    store: &Store,
    key: Hash,
) -> Result<(Module, Hash), DeserializeError> {
    let f = FileSystemCache::new(path)?;
    unsafe { f.load(store, key) }.map(|m| (m, key))
}

/// store a module into the cache
pub fn store_module<I: Into<PathBuf>>(
    path: I,
    module: &Module,
    bytes: &[u8],
) -> Result<Hash, SerializeError> {
    let mut cache = FileSystemCache::new(path)?;
    #[cfg(target_os = "windows")]
    {
        cache.set_cache_extension(Some("dll"))
    }
    let key = Hash::generate(bytes);

    cache.store(key, module)?;
    Ok(key)
}
