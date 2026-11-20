//! A store of complete Zig installs. An atomic rename publishes each install after its copy is complete.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::zig_build::{Build, STAMP};

/// The install key covers the pin, prefix identity, stamp, target, build arguments, libc description and host SDK.
pub fn key(prefix: &Path, pin: &str, build: &Build, libc: Option<&Path>, sdk: &[u8]) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut add = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    add(b"ghostty-vt-lib-v2");
    add(pin.as_bytes());
    add(prefix.file_name().ok_or("the prefix has no name")?.as_encoded_bytes());
    add(&read(&prefix.join(STAMP))?);
    add(build.zig_target.as_bytes());
    let mut args = build.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--prefix" || arg == "--cache-dir" || arg == "-j" {
            args.next();
        } else if !arg.to_string_lossy().starts_with("-j") {
            add(arg.as_encoded_bytes());
        }
    }
    add(if libc.is_some() { b"libc" } else { b"no-libc" });
    if let Some(path) = libc {
        add(&read(path)?);
    }
    add(sdk);
    Ok(hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The store is beside the prepared prefix in the vendor root.
pub fn path(prefix: &Path, key: &str) -> Result<PathBuf, String> {
    Ok(prefix
        .parent()
        .ok_or("the prefix has no parent")?
        .join(format!("ghostty-vt-lib-{key}")))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))
}

fn archive_relative(build: &Build) -> Result<PathBuf, String> {
    Ok(Path::new("lib").join(build.archive.file_name().ok_or("the archive has no name")?))
}

fn complete(store: &Path, build: &Build) -> bool {
    read(&store.join("complete")).is_ok_and(|bytes| bytes == b"ghostty-vt-lib-v1\n")
        && archive_relative(build).is_ok_and(|archive| {
            fs::metadata(store.join("install").join(archive)).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        })
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn install_dir(build: &Build) -> Result<&Path, String> {
    build
        .archive
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "the archive has no install directory".into())
}

fn restore(store: &Path, build: &Build) -> Result<bool, String> {
    if !complete(store, build) {
        return Ok(false);
    }
    let install = install_dir(build)?;
    if install.exists() {
        fs::remove_dir_all(install).map_err(|err| format!("cannot clear {}: {err}", install.display()))?;
    }
    copy_tree(&store.join("install"), install).map_err(|err| format!("cannot restore {}: {err}", store.display()))?;
    Ok(true)
}

fn publish(store: &Path, build: &Build) -> Result<(), String> {
    let result = (|| -> std::io::Result<()> {
        let parent = store
            .parent()
            .ok_or_else(|| std::io::Error::other("the store has no parent"))?;
        let stage = tempfile::Builder::new().prefix(".ghostty-vt-lib-").tempdir_in(parent)?;
        copy_tree(
            install_dir(build).map_err(std::io::Error::other)?,
            &stage.path().join("install"),
        )?;
        fs::write(stage.path().join("complete"), b"ghostty-vt-lib-v1\n")?;
        if store.exists() && !complete(store, build) {
            fs::remove_dir_all(store)?;
        }
        match fs::rename(stage.path(), store) {
            Ok(()) => Ok(()),
            Err(_) if complete(store, build) => Ok(()),
            Err(err) => Err(err),
        }
    })();
    result.map_err(|err| format!("cannot save {}: {err}", store.display()))
}

/// Restore an install or build and publish it. Return true on a hit, which never calls `compile`.
pub fn install(
    build: &Build,
    store: Option<&Path>,
    compile: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if let Some(store) = store {
        if restore(store, build)? {
            println!(
                "cargo::warning=libghostty-vt store hit: {} (Zig skipped)",
                store.display()
            );
            return Ok(true);
        }
        println!("cargo::warning=libghostty-vt store miss: {}", store.display());
    }
    let install = install_dir(build)?;
    if install.exists() {
        fs::remove_dir_all(install).map_err(|err| format!("cannot clear {}: {err}", install.display()))?;
    }
    compile()?;
    if !fs::metadata(&build.archive).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
        return Err(format!(
            "the libghostty-vt build did not produce {}",
            build.archive.display()
        ));
    }
    if let Some(store) = store {
        publish(store, build)?;
        println!("cargo::warning=libghostty-vt store saved: {}", store.display());
    }
    Ok(false)
}
