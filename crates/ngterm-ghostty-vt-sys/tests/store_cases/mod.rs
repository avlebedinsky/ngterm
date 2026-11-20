use super::*;

#[path = "../../lib_store.rs"]
mod lib_store;
#[path = "../../sdk.rs"]
mod sdk;

fn prefix_at(root: &Path) -> PathBuf {
    let prefix = root.join("ghostty-vt-0123456789abcdef");
    fs::create_dir_all(prefix.join("share/ngterm")).unwrap();
    fs::write(
        prefix.join(zig_build::STAMP),
        "zig=/zig\nzig-version=0.17.0\nzig-global-cache=/cache\nversion-string=1.4.0\npatch=fix.patch abc\n",
    )
    .unwrap();
    prefix
}

fn store_key(prefix: &Path, build: &Build) -> String {
    lib_store::key(prefix, build, None, b"sdk=27\n").unwrap()
}

#[test]
fn each_stamp_field_and_the_prefix_identity_change_the_key() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = plan(root.path(), "x86_64-unknown-linux-gnu", None);
    let stamp = fs::read_to_string(prefix.join(zig_build::STAMP)).unwrap();
    let base = store_key(&prefix, &build);
    for (from, to) in [
        ("zig=/zig", "zig=/other-zig"),
        ("zig-version=0.17.0", "zig-version=0.17.1"),
        ("zig-global-cache=/cache", "zig-global-cache=/new-cache"),
        ("version-string=1.4.0", "version-string=1.5.0"),
        ("patch=fix.patch abc", "patch=fix.patch def"),
    ] {
        fs::write(prefix.join(zig_build::STAMP), stamp.replace(from, to)).unwrap();
        assert_ne!(base, store_key(&prefix, &build), "{from}");
    }
    fs::write(prefix.join(zig_build::STAMP), &stamp).unwrap();
    let moved = root.path().join("ghostty-vt-new-pin");
    fs::rename(&prefix, &moved).unwrap();
    assert_ne!(base, store_key(&moved, &build));
}

#[test]
fn each_build_argument_and_target_change_the_key() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = plan(root.path(), "x86_64-unknown-linux-gnu", None);
    let base = store_key(&prefix, &build);
    for (i, arg) in build
        .args
        .iter()
        .enumerate()
        .filter(|(_, arg)| arg.to_string_lossy().starts_with("-D"))
    {
        let mut changed = build.clone();
        changed.args[i].push("-changed");
        assert_ne!(base, store_key(&prefix, &changed), "{arg:?}");
    }
    let mut changed = build.clone();
    changed.zig_target.push_str("-changed");
    assert_ne!(base, store_key(&prefix, &changed));
    changed = build.clone();
    changed.args.push("--new-build-option".into());
    assert_ne!(base, store_key(&prefix, &changed));
}

#[test]
fn install_paths_and_job_counts_do_not_change_the_key() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = plan(root.path(), "x86_64-unknown-linux-gnu", None);
    let base = store_key(&prefix, &build);
    let mut changed = plan(&root.path().join("another-out-dir"), "x86_64-unknown-linux-gnu", None);
    let cache = changed.args.iter().position(|arg| arg == "--cache-dir").unwrap();
    changed.args[cache + 1] = "/another-cache".into();
    changed.args.retain(|arg| !arg.to_string_lossy().starts_with("-j"));
    changed.args.extend(["-j".into(), "2".into()]);
    assert_eq!(base, store_key(&prefix, &changed));
    changed.args.pop();
    changed.args.pop();
    assert_eq!(base, store_key(&prefix, &changed));
}

#[test]
fn libc_contents_change_the_key_and_its_file_name_does_not() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = plan(root.path(), "x86_64-unknown-linux-gnu", None);
    let libc = root.path().join("libc.txt");
    fs::write(&libc, "include_dir=/sdk/one").unwrap();
    let base = lib_store::key(&prefix, &build, Some(&libc), b"sdk").unwrap();
    assert_ne!(base, lib_store::key(&prefix, &build, None, b"sdk").unwrap());
    fs::write(&libc, "include_dir=/sdk/two").unwrap();
    assert_ne!(base, lib_store::key(&prefix, &build, Some(&libc), b"sdk").unwrap());
    let other = root.path().join("other-libc.txt");
    fs::write(&other, "include_dir=/sdk/one").unwrap();
    assert_eq!(base, lib_store::key(&prefix, &build, Some(&other), b"sdk").unwrap());
    assert!(lib_store::key(&prefix, &build, Some(&root.path().join("missing")), b"sdk").is_err());
}

#[test]
fn each_sdk_version_changes_the_key() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = plan(root.path(), "x86_64-unknown-linux-gnu", None);
    let sdk = "ndk=27.1\napple-version=27.0\napple-build=AAA\nmsvc=14.44\nwindows-sdk=10.0.26100\n";
    let base = lib_store::key(&prefix, &build, None, sdk.as_bytes()).unwrap();
    for value in ["27.1", "27.0", "AAA", "14.44", "10.0.26100"] {
        let changed = sdk.replace(value, "changed");
        assert_ne!(
            base,
            lib_store::key(&prefix, &build, None, changed.as_bytes()).unwrap(),
            "{value}"
        );
    }
}

#[test]
fn ndk_properties_change_the_sdk_identity_at_the_same_path() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.properties");
    fs::write(&path, "Pkg.Revision = 27.1.12297006").unwrap();
    let before = sdk::fingerprint("aarch64-linux-android", Some(root.path())).unwrap();
    fs::write(&path, "Pkg.Revision = 28.0.13004108").unwrap();
    assert_ne!(
        before,
        sdk::fingerprint("aarch64-linux-android", Some(root.path())).unwrap()
    );
    fs::remove_file(path).unwrap();
    assert!(sdk::fingerprint("aarch64-linux-android", Some(root.path())).is_err());
    assert!(sdk::fingerprint("aarch64-linux-android", None).is_err());
}

#[test]
fn the_host_sdk_probe_covers_its_native_build_and_selection_variables() {
    let target = if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let sdk = sdk::fingerprint(target, None).unwrap();
    assert!(!sdk.is_empty());
    if cfg!(windows) {
        let text = String::from_utf8_lossy(&sdk);
        assert!(text.contains("sdk="), "{text}");
        assert!(text.contains("msvc="), "{text}");
    }
    for name in [
        "DEVELOPER_DIR",
        "SDKROOT",
        "VCToolsVersion",
        "WindowsSDKVersion",
        "LIB",
        "INCLUDE",
    ] {
        assert!(sdk::ENV.contains(&name));
    }
    assert_eq!(sdk::apple_sdk("aarch64-apple-darwin"), "macosx");
    assert_eq!(sdk::apple_sdk("aarch64-apple-ios"), "iphoneos");
    assert_eq!(sdk::apple_sdk("aarch64-apple-ios-sim"), "iphonesimulator");
}

fn fake_build(root: &Path) -> Build {
    let mut build = plan(root, "x86_64-unknown-linux-gnu", None);
    build.zig = script::bash().unwrap();
    build.source = root.to_path_buf();
    build.args = vec!["-c".into(), "exit 73".into()];
    build
}

fn compile_install(build: &Build) -> Result<(), String> {
    let install = build.archive.parent().unwrap().parent().unwrap();
    fs::create_dir_all(build.archive.parent().unwrap()).unwrap();
    fs::write(&build.archive, b"archive").unwrap();
    fs::create_dir_all(install.join("include")).unwrap();
    fs::write(install.join("include/vt.h"), b"header").unwrap();
    Ok(())
}

#[test]
fn a_complete_store_restores_the_install_without_a_zig_call() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = fake_build(root.path());
    let key = store_key(&prefix, &build);
    let store = lib_store::path(&prefix, &key).unwrap();
    assert!(zig_build::run(&build, None).is_err());
    lib_store::install(&build, Some(&store), || compile_install(&build)).unwrap();
    let install = root.path().join("zig-out");
    fs::remove_dir_all(&install).unwrap();
    let mut fresh = build.clone();
    fresh.archive = root.path().join("fresh/zig-out/lib/libghostty-vt.a");
    lib_store::install(&fresh, Some(&store), || zig_build::run(&fresh, None)).unwrap();
    assert_eq!(fs::read(&fresh.archive).unwrap(), b"archive");
    assert_eq!(
        fs::read(root.path().join("fresh/zig-out/include/vt.h")).unwrap(),
        b"header"
    );
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ghostty-vt-lib-")
    }));
}

#[test]
fn a_partial_store_or_failed_build_never_becomes_a_hit() {
    let root = tempfile::tempdir().unwrap();
    let prefix = prefix_at(root.path());
    let build = fake_build(root.path());
    let store = lib_store::path(&prefix, &store_key(&prefix, &build)).unwrap();
    fs::create_dir_all(store.join("install/lib")).unwrap();
    fs::write(store.join("install/lib/libghostty-vt.a"), b"partial").unwrap();
    assert!(lib_store::install(&build, Some(&store), || zig_build::run(&build, None)).is_err());
    assert!(!store.join("complete").exists());
    fs::write(store.join("complete"), "ghostty-vt-lib-v1\n").unwrap();
    fs::write(store.join("install/lib/libghostty-vt.a"), b"").unwrap();
    assert!(lib_store::install(&build, Some(&store), || zig_build::run(&build, None)).is_err());
    lib_store::install(&build, Some(&store), || compile_install(&build)).unwrap();
    lib_store::install(&build, Some(&store), || zig_build::run(&build, None)).unwrap();
    assert_eq!(fs::read(&build.archive).unwrap(), b"archive");
}

#[test]
fn a_build_without_an_archive_never_publishes_and_a_disabled_store_builds() {
    let root = tempfile::tempdir().unwrap();
    let build = fake_build(root.path());
    let store = root.path().join("ghostty-vt-lib-key");
    assert!(lib_store::install(&build, Some(&store), || Ok(())).is_err());
    assert!(!store.exists());
    assert!(lib_store::install(&build, None, || zig_build::run(&build, None)).is_err());
    lib_store::install(&build, None, || compile_install(&build)).unwrap();
    assert!(!store.exists());
}

#[test]
fn concurrent_builds_publish_one_complete_store() {
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("ghostty-vt-lib-concurrent");
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let handles: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|name| {
                let build = fake_build(&root.path().join(name));
                let store = &store;
                let barrier = &barrier;
                scope.spawn(move || {
                    lib_store::install(&build, Some(store), || {
                        barrier.wait();
                        compile_install(&build)
                    })
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert_eq!(fs::read(store.join("install/lib/libghostty-vt.a")).unwrap(), b"archive");
    assert_eq!(fs::read(store.join("install/include/vt.h")).unwrap(), b"header");
    let restored = fake_build(&root.path().join("restored"));
    lib_store::install(&restored, Some(&store), || zig_build::run(&restored, None)).unwrap();
}
