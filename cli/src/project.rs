//! Create a self-contained, pinned contract starter without running Cargo.

use crate::util::write_new;
use clap::Args;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(crate) struct NewArgs {
    /// New directory to create; existing directories are never modified.
    directory: PathBuf,
    /// Cargo package name; defaults to the directory's final component.
    #[arg(long)]
    name: Option<String>,
}

const LOCKFILE: &str = include_str!("../templates/starter/Cargo.lock.in");

const TEXT_FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("../templates/starter/Cargo.toml.in"),
    ),
    ("Cargo.lock", LOCKFILE),
    (
        "rust-toolchain.toml",
        include_str!("../../rust-toolchain.toml"),
    ),
    ("README.md", include_str!("../templates/starter/README.md")),
    (
        "src/contract.rs",
        include_str!("../templates/starter/src/contract.rs"),
    ),
    (
        "src/main.rs",
        include_str!("../templates/starter/src/main.rs"),
    ),
    (
        "src/tests.rs",
        include_str!("../templates/starter/src/tests.rs"),
    ),
    ("LICENSE", include_str!("../../LICENSE")),
    (".gitignore", "/target/\n/demo/\n/rejected/\n"),
];

fn package_name(args: &NewArgs) -> Result<&str, Box<dyn Error>> {
    let name = args
        .name
        .as_deref()
        .or_else(|| {
            args.directory
                .file_name()
                .and_then(|component| component.to_str())
        })
        .ok_or("cannot infer a package name; supply --name")?;
    let mut bytes = name.bytes();
    if name.len() > 64
        || !bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("package name must start with an ASCII letter and contain at most 64 letters, digits, '_' or '-'".into());
    }
    if matches!(name, "build" | "deps" | "examples" | "incremental") {
        return Err(
            format!("package name '{name}' conflicts with a Cargo output directory").into(),
        );
    }
    // The pinned lockfile uses unqualified dependency names when they are
    // unique. Adding another package with one of those names would require
    // Cargo to rewrite the lockfile before the documented --locked build.
    if LOCKFILE.lines().any(|line| {
        line.strip_prefix("name = \"")
            .and_then(|name| name.strip_suffix('"'))
            == Some(name)
    }) {
        return Err(format!(
            "package name '{name}' conflicts with a pinned dependency; choose a different --name"
        )
        .into());
    }
    Ok(name)
}

pub(crate) fn run(args: NewArgs) -> Result<(), Box<dyn Error>> {
    let name = package_name(&args)?;
    if let Some(parent) = args
        .directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&args.directory).map_err(|error| {
        format!(
            "cannot create project '{}': {error}; choose a new directory",
            args.directory.display()
        )
    })?;
    fs::create_dir(args.directory.join("src"))?;
    for (path, source) in TEXT_FILES {
        let text = source.replace("@@PACKAGE_NAME@@", name);
        write_new(&args.directory.join(path), text.as_bytes())?;
    }
    write_new(
        &args.directory.join("pay_at_least.wasm"),
        include_bytes!("../../evaluators/artifacts/pay_at_least.wasm"),
    )?;
    println!("{}", args.directory.display());
    eprintln!("Created {name}. Follow its README.md to build and complete a synthetic payment.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unusable_starter_names_fail_before_creating_directories() {
        let directory = std::env::temp_dir().join(format!(
            "sapio-invalid-starter-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        for name in [
            "bitcoin",
            "serde",
            "build",
            "deps",
            "examples",
            "incremental",
        ] {
            let error = run(NewArgs {
                directory: directory.join("project"),
                name: Some(name.into()),
            })
            .unwrap_err();
            assert!(error.to_string().contains(name));
            assert!(!directory.exists());
        }
    }

    #[test]
    fn package_names_are_inferred_or_explicit_without_keyword_restrictions() {
        for (directory, explicit, expected) in [
            ("my-contract", None, "my-contract"),
            (
                "directory with spaces",
                Some("bitcoin-contract"),
                "bitcoin-contract",
            ),
            ("fn", None, "fn"),
            ("test", None, "test"),
        ] {
            let args = NewArgs {
                directory: directory.into(),
                name: explicit.map(Into::into),
            };
            assert_eq!(package_name(&args).unwrap(), expected);
        }
    }
}
