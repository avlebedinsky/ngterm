//! Host SDK identities for the finished library store. Unknown SDKs disable the store for that build.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// SDK selection variables whose changes must rerun the build script.
pub const ENV: &[&str] = &[
    "DEVELOPER_DIR",
    "SDKROOT",
    "LOCALAPPDATA",
    "VCToolsInstallDir",
    "VCToolsVersion",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "UniversalCRTSdkDir",
    "UCRTVersion",
    "VSINSTALLDIR",
    "VCINSTALLDIR",
    "LIB",
    "INCLUDE",
    "ZIG_LIB_DIR",
];

fn record(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    println!("cargo::rerun-if-changed={}", path.display());
    std::fs::read(path).map_err(|err| format!("cannot read SDK input {}: {err}", path.display()))
}

fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("cannot run {program}: {err}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(format!(
            "{program} cannot identify the SDK: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// The Apple SDK name follows Zig's `std.zig.system.darwin.getSdk`.
pub fn apple_sdk(target: &str) -> &'static str {
    if target.ends_with("-ios-sim") {
        "iphonesimulator"
    } else if target.ends_with("-ios") {
        "iphoneos"
    } else {
        "macosx"
    }
}

/// Read the NDK revision, Apple SDK settings, or the installed MSVC and Windows SDK versions.
pub fn fingerprint(target: &str, ndk: Option<&Path>, libc: Option<&Path>) -> Result<Vec<u8>, String> {
    fingerprint_with_env(target, ndk, libc, |name| std::env::var_os(name))
}

fn fingerprint_with_env(
    target: &str,
    ndk: Option<&Path>,
    libc: Option<&Path>,
    mut env: impl FnMut(&str) -> Option<OsString>,
) -> Result<Vec<u8>, String> {
    if libc.is_some() {
        return Err("the SDK identity of an explicit ZIG_LIBC is unknown".into());
    }
    let mut bytes = Vec::new();
    record(&mut bytes, std::env::consts::OS.as_bytes());
    record(&mut bytes, std::env::consts::ARCH.as_bytes());
    for name in ENV {
        record(&mut bytes, name.as_bytes());
        record(&mut bytes, env(name).unwrap_or_default().as_encoded_bytes());
    }
    if target.ends_with("-android") {
        let ndk = ndk.ok_or("the Android NDK path is unknown")?;
        record(&mut bytes, ndk.as_os_str().as_encoded_bytes());
        record(&mut bytes, &read(&ndk.join("source.properties"))?);
    }
    if target.contains("-apple-") && cfg!(target_os = "macos") {
        record(&mut bytes, &apple_inputs(target, run)?);
    }
    if target.ends_with("-windows-msvc") && cfg!(windows) {
        let output = run(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("windows_sdk.ps1"),
            ],
        )?;
        for line in String::from_utf8_lossy(&output).lines() {
            if let Some(path) = line.strip_prefix("watch=") {
                println!("cargo::rerun-if-changed={path}");
            }
        }
        record(&mut bytes, &output);
    }
    Ok(bytes)
}

fn apple_inputs(
    target: &str,
    mut probe: impl FnMut(&str, &[&str]) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let sdk = apple_sdk(target);
    let path = probe("xcrun", &["--sdk", sdk, "--show-sdk-path"])?;
    record(&mut bytes, &path);
    record(&mut bytes, &probe("xcrun", &["--sdk", sdk, "--show-sdk-version"])?);
    record(
        &mut bytes,
        &probe("xcrun", &["--sdk", sdk, "--show-sdk-build-version"])?,
    );
    let path = String::from_utf8(path).map_err(|err| format!("the SDK path is not UTF-8: {err}"))?;
    record(&mut bytes, &read(&Path::new(path.trim()).join("SDKSettings.json"))?);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sdk_selection_value_changes_the_fingerprint() {
        let fingerprint = |selection: Option<(&str, &str)>| {
            fingerprint_with_env("x86_64-unknown-linux-gnu", None, None, |name| {
                selection.filter(|(key, _)| *key == name).map(|(_, value)| value.into())
            })
            .unwrap()
        };
        let absent = fingerprint(None);
        assert_eq!(absent, fingerprint(None));
        for name in ENV {
            let first = fingerprint(Some((name, "sdk-one")));
            assert_ne!(absent, first, "{name}");
            assert_ne!(first, fingerprint(Some((name, "sdk-two"))), "{name}");
            assert_eq!(absent, fingerprint(Some((name, ""))), "{name}");
        }
    }

    #[test]
    fn an_explicit_libc_disables_the_store_when_its_sdk_identity_is_unknown() {
        let root = tempfile::tempdir().unwrap();
        let libc = root.path().join("libc.txt");
        std::fs::write(&libc, "include_dir=/portable-sdk/include\n").unwrap();
        let result = fingerprint("x86_64-unknown-linux-gnu", None, Some(&libc));
        assert!(result.unwrap_err().contains("explicit ZIG_LIBC"));
        assert!(fingerprint("x86_64-unknown-linux-gnu", None, None).is_ok());
    }

    fn apple(path: &Path, version: &str, build: &str) -> Vec<u8> {
        apple_inputs("aarch64-apple-ios-sim", |program, args| {
            assert_eq!(program, "xcrun");
            assert_eq!(&args[..2], &["--sdk", "iphonesimulator"]);
            Ok(match args[2] {
                "--show-sdk-path" => path.as_os_str().as_encoded_bytes().to_vec(),
                "--show-sdk-version" => version.as_bytes().to_vec(),
                "--show-sdk-build-version" => build.as_bytes().to_vec(),
                other => panic!("unexpected xcrun argument: {other}"),
            })
        })
        .unwrap()
    }

    #[test]
    fn apple_sdk_path_version_build_and_settings_change_the_identity() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("SDKSettings.json");
        std::fs::write(&settings, b"sdk settings one").unwrap();
        let base = apple(root.path(), "27.0", "AAA");
        assert_ne!(base, apple(root.path(), "27.1", "AAA"));
        assert_ne!(base, apple(root.path(), "27.0", "BBB"));
        std::fs::write(&settings, b"sdk settings two").unwrap();
        assert_ne!(base, apple(root.path(), "27.0", "AAA"));
        let moved = root.path().join("other-sdk");
        std::fs::create_dir(&moved).unwrap();
        std::fs::write(moved.join("SDKSettings.json"), b"sdk settings one").unwrap();
        assert_ne!(base, apple(&moved, "27.0", "AAA"));
        assert!(apple_inputs("aarch64-apple-ios", |_, _| Err("no SDK".into())).is_err());
    }
}
