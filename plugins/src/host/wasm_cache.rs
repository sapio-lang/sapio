// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Content-addressed WASM source storage. Every load compiles with its caller's
//! engine; cached native executables are never deserialized.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use wasmer::{Module, Store};
use wasmer_cache::Hash;

/// Maximum binary WASM source size, including debug information.
pub const MAX_MODULE_BYTES: usize = 128 * 1024 * 1024;

const WASM_HEADER: &[u8] = b"\0asm\x01\0\0\0";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

fn invalid_source(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn check_source(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_MODULE_BYTES {
        return Err(invalid_source("WASM source exceeds the module size limit"));
    }
    if !bytes.starts_with(WASM_HEADER) {
        return Err(invalid_source("Expected a binary WASM module header"));
    }
    Ok(())
}

fn source_path(path: &Path, key: Hash) -> PathBuf {
    path.join("sources").join(format!("{key}.wasm"))
}

fn read_source(path: &Path, key: Hash) -> io::Result<Vec<u8>> {
    // Reject non-files before opening, so directories and ordinary named pipes
    // cannot act as source entries. Hash verification still authenticates bytes.
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(invalid_source(
            "WASM source cache entry is not a regular file",
        ));
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_MODULE_BYTES as u64 {
        return Err(invalid_source(
            "WASM source cache entry exceeds the module size limit",
        ));
    }
    let mut bytes = Vec::new();
    // The file may grow after the metadata check. Read at most one byte beyond
    // the limit so growth cannot make the allocation unbounded.
    file.take(MAX_MODULE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    check_source(&bytes)?;
    if Hash::generate(&bytes) != key {
        return Err(invalid_source(
            "WASM source does not match the requested cache key",
        ));
    }
    Ok(bytes)
}

/// List authenticated source keys in deterministic order.
///
/// A missing source directory is empty. Legacy executable caches and unrelated
/// files are ignored; I/O failures and corruption of source entries are errors.
/// Full WASM validation occurs when loading with the caller's engine.
pub fn get_all_keys_from_fs<I: Into<PathBuf>>(
    path: I,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let directory = path.into().join("sources");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut keys = Vec::new();
    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(name) = filename
            .to_str()
            .and_then(|name| name.strip_suffix(".wasm"))
        else {
            continue;
        };
        let Ok(key) = Hash::from_str(name) else {
            continue;
        };
        if key.to_string() != name {
            continue;
        }
        read_source(&entry.path(), key)?;
        keys.push(name.to_owned());
    }
    keys.sort_unstable();
    Ok(keys)
}

/// Compile binary WASM with the supplied store and cache its exact source.
///
/// Existing source entries must be intact. Compilation and cache errors are
/// propagated rather than treated as cache misses.
pub fn load_module<I: Into<PathBuf>>(
    path: I,
    store: &Store,
    bytes: &[u8],
) -> Result<(Module, Hash), Box<dyn std::error::Error>> {
    check_source(bytes)?;
    let key = Hash::generate(bytes);
    let module = Module::new(store, bytes)?;
    let source = source_path(&path.into(), key);
    match read_source(&source, key) {
        Ok(_) => return Ok((module, key)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let directory = source.parent().expect("source path has a parent");
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(
        ".{key}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        drop(file);
        // Publish complete sources so concurrent readers never see our partial
        // writes. Writers for the same content key publish identical bytes.
        fs::rename(&temporary, &source)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok((module, key))
}

/// Authenticate cached source and compile it with the supplied store.
///
/// Native executable cache entries are never consulted. A new store's compiler
/// configuration therefore applies even when this source was previously used.
pub fn load_module_key<I: Into<PathBuf>>(
    path: I,
    store: &Store,
    key: Hash,
) -> Result<(Module, Hash), Box<dyn std::error::Error>> {
    let bytes = read_source(&source_path(&path.into(), key), key)?;
    Ok((Module::new(store, &bytes)?, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmer::{imports, Instance};

    struct TempCache(PathBuf);

    impl TempCache {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sapio-wasm-source-cache-{}-{}",
                std::process::id(),
                NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write_source(&self, key: Hash, bytes: &[u8]) {
            fs::create_dir_all(self.0.join("sources")).unwrap();
            fs::write(source_path(&self.0, key), bytes).unwrap();
        }
    }

    impl Drop for TempCache {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn source(value: i32) -> Vec<u8> {
        wasmer::wat2wasm(
            format!("(module (func (export \"answer\") (result i32) i32.const {value}))")
                .as_bytes(),
        )
        .unwrap()
        .into_owned()
    }

    fn answer(module: &Module, store: &mut Store) -> i32 {
        Instance::new(store, module, &imports! {})
            .unwrap()
            .exports
            .get_typed_function::<(), i32>(store, "answer")
            .unwrap()
            .call(store)
            .unwrap()
    }

    #[test]
    fn source_roundtrip_preserves_bytes_and_runs_in_fresh_stores() {
        let cache = TempCache::new();
        let bytes = source(42);
        let mut first_store = Store::default();
        let (module, key) = load_module(&cache.0, &first_store, &bytes).unwrap();
        assert_eq!(key, Hash::generate(&bytes));
        assert_eq!(fs::read(source_path(&cache.0, key)).unwrap(), bytes);
        assert_eq!(answer(&module, &mut first_store), 42);
        let mut second_store = Store::default();
        let (loaded, loaded_key) = load_module_key(&cache.0, &second_store, key).unwrap();
        assert_eq!(loaded_key, key);
        assert_eq!(answer(&loaded, &mut second_store), 42);
        let (_, repeated_key) = load_module(&cache.0, &second_store, &bytes).unwrap();
        assert_eq!(repeated_key, key);
        let (_, other_key) = load_module(&cache.0, &second_store, &source(7)).unwrap();
        let mut expected_keys = vec![key.to_string(), other_key.to_string()];
        expected_keys.sort_unstable();
        assert_eq!(get_all_keys_from_fs(&cache.0).unwrap(), expected_keys);
    }

    #[test]
    fn mismatched_source_is_an_error_and_is_not_silently_replaced() {
        let cache = TempCache::new();
        let bytes = source(42);
        let key = Hash::generate(&bytes);
        let changed = source(43);
        cache.write_source(key, &changed);
        let store = Store::default();
        for error in [
            load_module_key(&cache.0, &store, key).unwrap_err(),
            load_module(&cache.0, &store, &bytes).unwrap_err(),
            get_all_keys_from_fs(&cache.0).unwrap_err(),
        ] {
            assert!(error.to_string().contains("does not match"));
        }
        assert_eq!(fs::read(source_path(&cache.0, key)).unwrap(), changed);
    }

    #[test]
    fn truncated_wasm_is_rejected_even_under_its_own_content_key() {
        let cache = TempCache::new();
        let mut bytes = source(42);
        bytes.pop();
        let key = Hash::generate(&bytes);
        let store = Store::default();
        assert!(load_module(&cache.0, &store, &bytes).is_err());
        assert!(get_all_keys_from_fs(&cache.0).unwrap().is_empty());
        cache.write_source(key, &bytes);
        assert!(load_module_key(&cache.0, &store, key)
            .unwrap_err()
            .downcast_ref::<wasmer::CompileError>()
            .is_some());
    }

    #[test]
    fn oversized_sources_are_rejected_before_reading_or_writing() {
        let cache = TempCache::new();
        let bytes = source(42);
        let key = Hash::generate(&bytes);
        cache.write_source(key, &bytes);
        File::options()
            .write(true)
            .open(source_path(&cache.0, key))
            .unwrap()
            .set_len(MAX_MODULE_BYTES as u64 + 1)
            .unwrap();
        let store = Store::default();
        assert!(load_module_key(&cache.0, &store, key)
            .unwrap_err()
            .to_string()
            .contains("size limit"));
        assert!(get_all_keys_from_fs(&cache.0)
            .unwrap_err()
            .to_string()
            .contains("size limit"));
        let oversized = vec![0; MAX_MODULE_BYTES + 1];
        let empty_cache = TempCache::new();
        assert!(load_module(&empty_cache.0, &store, &oversized)
            .unwrap_err()
            .to_string()
            .contains("size limit"));
        assert!(!empty_cache.0.join("sources").exists());
    }

    #[test]
    fn legacy_executables_and_unrelated_files_are_not_source_entries() {
        let cache = TempCache::new();
        let bytes = source(42);
        let key = Hash::generate(&bytes);
        let store = Store::default();
        let native = Module::new(&store, &bytes).unwrap().serialize().unwrap();
        fs::write(cache.0.join(key.to_string()), &native).unwrap();
        fs::write(cache.0.join(format!("{key}.dll")), &native).unwrap();
        assert!(get_all_keys_from_fs(&cache.0).unwrap().is_empty());
        assert_eq!(
            load_module_key(&cache.0, &store, key)
                .unwrap_err()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::NotFound
        );
        load_module(&cache.0, &store, &bytes).unwrap();
        let directory = cache.0.join("sources");
        fs::write(directory.join("notes.txt"), &bytes).unwrap();
        fs::write(directory.join("not-a-key.wasm"), &bytes).unwrap();
        fs::write(directory.join(format!(".{key}.tmp")), &bytes).unwrap();
        fs::create_dir(directory.join("unrelated-directory")).unwrap();
        assert_eq!(
            get_all_keys_from_fs(&cache.0).unwrap(),
            vec![key.to_string()]
        );
        assert_eq!(
            fs::read(cache.0.join(key.to_string())).unwrap(),
            native.as_ref()
        );
    }

    #[test]
    fn native_bytes_and_text_are_rejected_without_creating_source_entries() {
        let cache = TempCache::new();
        let store = Store::default();
        let native = Module::new(&store, source(42))
            .unwrap()
            .serialize()
            .unwrap();
        for bytes in [native.as_ref(), b"(module)".as_slice(), b"\0asm".as_slice()] {
            assert!(load_module(&cache.0, &store, bytes)
                .unwrap_err()
                .to_string()
                .contains("binary WASM"));
            let key = Hash::generate(bytes);
            cache.write_source(key, bytes);
            assert!(load_module_key(&cache.0, &store, key).is_err());
            fs::remove_file(source_path(&cache.0, key)).unwrap();
        }
        assert!(get_all_keys_from_fs(&cache.0).unwrap().is_empty());
    }

    #[test]
    fn invalid_cache_directory_is_an_error_instead_of_a_miss() {
        let cache = TempCache::new();
        fs::write(cache.0.join("sources"), b"not a directory").unwrap();
        let bytes = source(42);
        let store = Store::default();
        assert!(get_all_keys_from_fs(&cache.0).is_err());
        assert!(load_module_key(&cache.0, &store, Hash::generate(&bytes)).is_err());
        assert!(load_module(&cache.0, &store, &bytes).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_source_is_not_treated_as_a_cache_miss() {
        use std::os::unix::fs::PermissionsExt;

        let cache = TempCache::new();
        let bytes = source(42);
        let key = Hash::generate(&bytes);
        cache.write_source(key, &bytes);
        let path = source_path(&cache.0, key);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
        // A privileged test runner may bypass Unix file permissions.
        if File::open(&path).is_ok() {
            return;
        }
        let store = Store::default();
        for error in [
            load_module_key(&cache.0, &store, key).unwrap_err(),
            load_module(&cache.0, &store, &bytes).unwrap_err(),
            get_all_keys_from_fs(&cache.0).unwrap_err(),
        ] {
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }
}
