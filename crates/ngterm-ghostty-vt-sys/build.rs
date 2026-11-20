//! Builds libghostty-vt for cargo's target with Zig and links the static archive.
//!
//! The prefix comes from `NGTERM_GHOSTTY_VT_PREFIX` when set, else from `scripts/build-ghostty-vt.sh --print-prefix`,
//! the one place that knows how it is keyed, run with bash (Git Bash on Windows, found explicitly, with the printed
//! MSYS path converted by `cygpath -m`: `script.rs`). The script prepares it (the pinned, patched source tree with
//! its Zig packages, headers, resources and stamp). A complete install from `lib_store` supplies `OUT_DIR` on a hit.
//! On a miss, this script runs `zig build -Demit-lib-vt` over that tree (`zig_build.rs`, from herdr's build script).
//! It installs into `OUT_DIR` and uses the stamp's Zig cache. It never reaches the network.
//! If the prefix is absent, the build fails with a pointer to the script.
//!
//! The archive needs no system library beyond the C runtime Rust links anyway: on Linux and macOS
//! (`libghostty-vt-static.pc` lists no `Libs.private` at `LIBGHOSTTY_VT_REV`) and on Windows MSVC, where
//! `ghostty-vt-static.lib` links alone (the link test, on the `windows-2025` runner and in the Win11 VM). On Apple
//! the archive is linked from a directory of its own, built for the deployment target rustc links for (macOS's, or
//! iOS's on device and simulator); each member's `LC_BUILD_VERSION` is checked against it, for macOS by the link test,
//! for iOS here after the build (with `otool -l` on the host, since the simulator can't run it). On Android (built
//! through cargo-ndk) the archive is built for cargo-ndk's API level, at least 24, against the NDK cargo-ndk links
//! with: Ghostty's build writes Zig's libc file for that NDK's bionic sysroot, configured again in each build.

mod lib_store;
mod script;
mod sdk;
mod zig_build;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// A failure exits non-zero, so cargo shows what the script printed beside the error: zig's own output when its
/// build fails (cargo hides a build script's output while it exits successfully, even with `cargo::error`).
fn main() -> ExitCode {
    match build() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            println!("cargo::error={message}");
            ExitCode::FAILURE
        }
    }
}

fn build() -> Result<(), String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?;
    let root = Path::new(&manifest).join("../..");
    for input in zig_build::RERUN_FILES {
        println!("cargo::rerun-if-changed={}", root.join(input).display());
    }
    for name in zig_build::RERUN_ENV.iter().chain(sdk::ENV) {
        println!("cargo::rerun-if-env-changed={name}");
    }
    let prefix = script::prefix(&root)?;
    println!("cargo::rerun-if-changed={}", prefix.join(zig_build::STAMP).display());
    let stamp = zig_build::prepared(&prefix)?;
    let read =
        |path: &Path| std::fs::read_to_string(path).map_err(|err| format!("cannot read {}: {err}", path.display()));
    let pin = zig_build::pin(
        &read(&root.join("build/pins.env"))?,
        &read(&prefix.join(zig_build::STAMP))?,
    )?;
    // the pin a session server's handoff compares (`ngterm_ghostty_vt_sys::PIN`)
    println!("cargo::rustc-env=NGTERM_GHOSTTY_VT_PIN={pin}");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    let target = std::env::var("TARGET").map_err(|err| format!("TARGET: {err}"))?;
    let macos_deployment_target = std::env::var("MACOSX_DEPLOYMENT_TARGET").ok();
    let ios_deployment_target = std::env::var("IPHONEOS_DEPLOYMENT_TARGET").ok();
    let jobs = std::env::var("NUM_JOBS").ok();
    let android_platform = std::env::var("ANDROID_PLATFORM").ok();
    let android_ndk = zig_build::android_ndk(
        std::env::var("ANDROID_NDK_HOME").ok().as_deref(),
        std::env::var("CARGO_NDK_SYSROOT_PATH").ok().as_deref(),
    );
    let build = zig_build::plan(&zig_build::Inputs {
        prefix: &prefix,
        stamp: &stamp,
        out_dir: &out_dir,
        target: &target,
        macos_deployment_target: macos_deployment_target.as_deref(),
        ios_deployment_target: ios_deployment_target.as_deref(),
        android_platform: android_platform.as_deref(),
        android_ndk: android_ndk.as_deref(),
        jobs: jobs.as_deref(),
    })?;

    let libc = std::env::var_os("ZIG_LIBC").filter(|path| !path.is_empty());
    if let Some(path) = &libc {
        println!("cargo::rerun-if-changed={}", Path::new(path).display());
    }
    let sdk = sdk::fingerprint(&target, android_ndk.as_deref());
    let store = match sdk {
        Ok(sdk) => Some(lib_store::path(
            &prefix,
            &lib_store::key(&prefix, &build, libc.as_deref().map(Path::new), &sdk)?,
        )?),
        Err(err) => {
            println!("cargo::warning=libghostty-vt store disabled: {err}");
            None
        }
    };
    lib_store::install(&build, store.as_deref(), || {
        check_zig(&stamp)?;
        zig_build::run(&build, libc.as_deref())?;
        if let Some((platform, deployment)) = zig_build::ios_build_version(&target, &build.zig_target) {
            check_ios_archive(&build.archive, platform, &deployment)?;
        }
        Ok(())
    })?;
    if let Some((platform, deployment)) = zig_build::ios_build_version(&target, &build.zig_target) {
        check_ios_archive(&build.archive, platform, &deployment)?;
    }
    let name = build.archive.file_name().ok_or("the archive has no file name")?;
    if build.archive.parent() != Some(build.link_dir.as_path()) {
        std::fs::create_dir_all(&build.link_dir)
            .map_err(|err| format!("cannot create {}: {err}", build.link_dir.display()))?;
        std::fs::copy(&build.archive, build.link_dir.join(name)).map_err(|err| {
            format!(
                "cannot copy {} into {}: {err}",
                build.archive.display(),
                build.link_dir.display()
            )
        })?;
    }
    println!("cargo::rustc-link-search=native={}", build.link_dir.display());
    println!("cargo::rustc-link-lib=static={}", build.link_name);
    // for the link test, which checks what the archive was built for
    println!(
        "cargo::rustc-env=NGTERM_GHOSTTY_VT_ARCHIVE={}",
        build.link_dir.join(name).display()
    );
    println!("cargo::rustc-env=NGTERM_GHOSTTY_VT_ZIG_TARGET={}", build.zig_target);
    Ok(())
}

/// An iOS archive carries the platform of its target and no newer iOS than it was built for, read with `otool -l` on
/// the (macOS) host, since no test running inside the simulator can (`zig_build::check_ios_archive`).
fn check_ios_archive(archive: &Path, platform: u32, deployment: &str) -> Result<(), String> {
    let output = Command::new("otool")
        .arg("-l")
        .arg(archive)
        .output()
        .map_err(|err| format!("cannot run otool on {}: {err}", archive.display()))?;
    if !output.status.success() {
        return Err(format!(
            "otool -l {} failed ({}): {}",
            archive.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    zig_build::check_ios_archive(&String::from_utf8_lossy(&output.stdout), platform, deployment)
        .map_err(|err| format!("{}: {err}", archive.display()))
}

/// The stamp's zig must still be the pinned one: the prefix was prepared with it, and an unpinned zig never builds.
fn check_zig(stamp: &zig_build::Stamp) -> Result<(), String> {
    let output = Command::new(&stamp.zig).arg("version").output();
    let version = output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    if version.as_deref() == Some(stamp.zig_version.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "{} is no longer zig {}: run scripts/build-ghostty-vt.sh (it prepares the prefix with the pinned zig)",
            stamp.zig.display(),
            stamp.zig_version
        ))
    }
}
