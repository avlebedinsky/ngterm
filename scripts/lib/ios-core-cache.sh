# shellcheck shell=bash
# A content-keyed cache of prebuilt iOS cores, for scripts/build-ios.sh: one entry per key, each the
# NgtermCoreFFI.xcframework and the NgtermCoreBindings.swift that steps 3 to 6 of build-ios.sh made. Two scratch copies
# of the repository on one Mac with the same core inputs then build the core once. Sourced after lib/native.sh, not
# run. Portable shell (stock macOS's bash 3.2, Linux's bash), as lib/native.sh is.
#
# The key covers what the core's build reads: every file of the local packages the core package depends on (its
# normal and build dependencies, all features, from `cargo metadata`), each file outside of them that their Rust
# sources take in with include_str! or include_bytes! of a literal path, the workspace manifest and lock, the toolchain
# files, the libghostty-vt pins and patches, the build scripts, the cargo profile, the toolchain's versions and the
# environment that changes a cargo build. A change outside of these (the Swift app, a desktop crate) keeps the key.
#
# The cache has a size limit: a store first removes the entries used least recently until the new one fits, and it
# stores nothing when the disk would then have less than the minimum free space. The caller holds the iOS build lock,
# so only one store or prune runs at a time.

# The files of the core build that are outside of the packages, relative to the repository root.
IOS_CORE_CACHE_FILES=(Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml build/pins.env scripts/build-ios.sh
  scripts/build-ghostty-vt.sh scripts/lib/native.sh scripts/lib/ios-core-cache.sh)

# The environment variables that change what a cargo build makes (the build inputs of
# crates/ngterm-ghostty-vt-sys/zig_build.rs's RERUN_ENV that an iOS build reads among them).
IOS_CORE_CACHE_ENV='^(RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_PROFILE_[A-Z0-9_]+|CARGO_BUILD_[A-Z0-9_]+|CARGO_TARGET_AARCH64_APPLE_IOS[A-Z0-9_]*|IPHONEOS_DEPLOYMENT_TARGET|MACOSX_DEPLOYMENT_TARGET|ZIG_LIBC|NGTERM_VENDOR|NGTERM_GHOSTTY_VT_PREFIX)='

# Reads `cargo metadata` on stdin and prints the directories (relative to the workspace root) of the local packages
# that the package $IOS_CORE_PACKAGE needs to build, itself included: dev-dependencies are not followed.
IOS_CORE_CLOSURE_PY='
import json
import os
import sys

meta = json.load(sys.stdin)
root = meta["workspace_root"]
packages = {package["id"]: package for package in meta["packages"]}
nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
todo = [i for i, package in packages.items() if package["name"] == os.environ["IOS_CORE_PACKAGE"]]
if not todo:
    sys.exit("no package " + os.environ["IOS_CORE_PACKAGE"] + " in the workspace")
seen = set()
while todo:
    current = todo.pop()
    if current in seen:
        continue
    seen.add(current)
    for dep in nodes[current]["deps"]:
        if any(kind.get("kind") != "dev" for kind in dep["dep_kinds"]):
            todo.append(dep["pkg"])
dirs = set()
for current in seen:
    if packages[current]["source"] is None:
        directory = os.path.relpath(os.path.dirname(packages[current]["manifest_path"]), root)
        if directory == "." or directory.startswith(".."):
            sys.exit("the local package " + packages[current]["name"] + " is not below the workspace root")
        dirs.add(directory)
print("\n".join(sorted(dirs)))
'

# Reads the package directories of IOS_CORE_CLOSURE_PY on stdin and prints the files (relative to the workspace root
# $1) that the Rust sources below them take in with include_str! or include_bytes! of a literal path and that are
# outside of those directories: a change to them changes the core too.
IOS_CORE_INCLUDES_PY='
import os
import re
import sys

root = os.path.realpath(sys.argv[1])
dirs = [line for line in sys.stdin.read().splitlines() if line]
inside = [os.path.join(root, directory) + os.sep for directory in dirs]
include = re.compile(r"include_(?:str|bytes)!\(\s*\"([^\"\\]+)\"\s*\)")
found = set()
for directory in dirs:
    for top, subdirs, files in os.walk(os.path.join(root, directory)):
        subdirs[:] = [name for name in subdirs if name != "target"]
        for name in files:
            if not name.endswith(".rs"):
                continue
            source = os.path.join(top, name)
            with open(source, encoding="utf-8", errors="replace") as handle:
                text = handle.read()
            for path in include.findall(text):
                full = os.path.realpath(os.path.join(os.path.dirname(source), path))
                if os.path.isfile(full) and full.startswith(root + os.sep) and not any(
                    full.startswith(prefix) for prefix in inside
                ):
                    found.add(os.path.relpath(full, root))
print("\n".join(sorted(found)))
'

# `<sha256>  <path>` for each NUL-separated path on stdin: coreutils' sha256sum, else shasum (stock macOS).
sha256_files() {
  if command -v sha256sum >/dev/null; then
    xargs -0 sha256sum --
  else
    xargs -0 shasum -a 256 --
  fi
}

# Prints the cache key of the core that the package $2 in the workspace at $1 makes, with the cargo profile $3 and the
# toolchain text $4 (the versions of rustc and Xcode). Fails when cargo can't read the workspace offline: the caller
# then builds without the cache.
ios_core_cache_key() {
  local root="$1" package="$2" profile="$3" toolchain="$4" dirs includes file listing
  dirs="$(cd "$root" && cargo metadata --offline --all-features --format-version 1 2>/dev/null |
    IOS_CORE_PACKAGE="$package" python3 -c "$IOS_CORE_CLOSURE_PY")" || return 1
  [[ -n "$dirs" ]] || return 1
  includes="$(python3 -c "$IOS_CORE_INCLUDES_PY" "$root" <<<"$dirs")" || return 1
  listing="$(
    cd "$root" || exit 1
    {
      for file in "${IOS_CORE_CACHE_FILES[@]}"; do
        [[ ! -f "$file" ]] || printf '%s\0' "$file"
      done
      [[ ! -d build/patches ]] || find build/patches -type f -print0
      while IFS= read -r file; do
        [[ -z "$file" ]] || printf '%s\0' "$file"
      done <<<"$includes"
      while IFS= read -r file; do
        find "$file" -name target -prune -o -name .DS_Store -prune -o -type f -print0
      done <<<"$dirs"
    } | LC_ALL=C sort -z | sha256_files
  )" || return 1
  {
    printf '%s\n' "$listing"
    printf 'profile %s\n%s\n' "$profile" "$toolchain"
    env | LC_ALL=C grep -E "$IOS_CORE_CACHE_ENV" | LC_ALL=C sort || true
  } | sha256_of | cut -c1-32
}

# The entry of key $2 in the cache $1, printed when it is complete (and marked as just used), else a failure.
ios_core_cache_entry() {
  local entry="$1/$2"
  [[ -f "$entry/.complete" && -d "$entry/NgtermCoreFFI.xcframework" && -f "$entry/NgtermCoreBindings.swift" ]] ||
    return 1
  touch "$entry/.complete"
  printf '%s\n' "$entry"
}

# The size of the directories $@ in KiB.
size_kib() {
  du -sk "$@" | awk '{ total += $1 } END { print total + 0 }'
}

# The free space of the file system that holds $1, in KiB.
free_kib() {
  df -Pk "$1" | awk 'NR == 2 { print $4 }'
}

# Removes entries of the cache $1 (an absolute path), least recently used first, until the rest take at most $2 KiB;
# the entry $3 (a key) stays. Leftovers of a store that was stopped go first.
ios_core_cache_prune() {
  local cache="$1" limit="$2" keep="${3:-}" name total entries="" line size
  [[ "$cache" == /* && -d "$cache" ]] || return 0
  for name in "$cache"/.tmp-* "$cache"/*; do
    [[ -d "$name" ]] || continue
    if [[ "${name##*/}" == .tmp-* || ! -f "$name/.complete" ]]; then
      rm -rf "${cache:?}/${name##*/}"
      continue
    fi
    line="$(size_and_mtime_ns "$name/.complete")"
    entries="$entries${line#* } ${name##*/}"$'\n'
  done
  [[ -n "$entries" ]] || return 0
  total=0
  while read -r _ name; do
    [[ -n "$name" ]] || continue
    total=$((total + $(size_kib "$cache/$name")))
  done <<<"$entries"
  while ((total > limit)) && read -r _ name; do
    [[ -n "$name" && "$name" != "$keep" ]] || continue
    size="$(size_kib "$cache/$name")"
    rm -rf "${cache:?}/${name:?}"
    total=$((total - size))
    echo "build-ios.sh: removed the cached core $name ($((size / 1024)) MiB) to keep the cache under its limit" >&2
  done < <(LC_ALL=C sort -n <<<"$entries")
}

# Stores the XCFramework $3 and the bindings $4 as the entry of key $2 in the cache $1 (an absolute path), in at most
# $5 MiB for the whole cache and only when $6 MiB of the disk stay free after it. Says why when it stores nothing.
ios_core_cache_store() {
  local cache="$1" key="$2" xcframework="$3" bindings="$4" limit=$(($5 * 1024)) min_free=$(($6 * 1024)) size tmp
  local copy=(cp -R)
  [[ "$cache" == /* ]] || die "build-ios.sh: the core cache $cache is not an absolute path"
  [[ "$(uname -s)" != Darwin ]] || copy=(cp -cR) # an APFS clone: no second copy on the disk
  mkdir -p "$cache"
  size="$(size_kib "$xcframework" "$bindings")"
  if ((size > limit)); then
    echo "build-ios.sh: the core ($((size / 1024)) MiB) is larger than the core cache's limit: not cached" >&2
    return 0
  fi
  ios_core_cache_prune "$cache" $((limit - size))
  if (($(free_kib "$cache") - size < min_free)); then
    echo "build-ios.sh: less than $(($6 / 1024)) GiB of the disk would stay free: the core is not cached" >&2
    return 0
  fi
  tmp="$cache/.tmp-$key"
  rm -rf "${tmp:?}"
  mkdir -p "$tmp"
  "${copy[@]}" "$xcframework" "$tmp/NgtermCoreFFI.xcframework"
  "${copy[@]}" "$bindings" "$tmp/NgtermCoreBindings.swift"
  touch "$tmp/.complete"
  rm -rf "${cache:?}/${key:?}"
  mv "$tmp" "$cache/$key"
  ios_core_cache_prune "$cache" "$limit" "$key"
  echo "build-ios.sh: cached the core as $key" >&2
}
