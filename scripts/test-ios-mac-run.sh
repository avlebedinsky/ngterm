#!/usr/bin/env bash
# Tests scripts/ios-mac-run.sh on Linux against a stand-in Mac: a home directory of its own, reached through an `ssh`
# that runs the command locally (rsync's too) and a `lockf` over flock(1). It checks the arguments, the copy and its
# checksum (a file that drifted on the Mac is copied again), the removal of what the tree lost, the lock (held for
# the command, its wait bounded), the time limit, the exit status, the logs and the cleanup. It also tests the iOS
# build lock under an outer lockf (lib/native.sh) and the core cache (lib/ios-core-cache.sh). Needs rsync, flock,
# perl, python3, git and cargo.
#
# Usage: scripts/test-ios-mac-run.sh
# shellcheck disable=SC2016,SC2034 # check() takes its condition as text and evaluates it later, with the variables it reads
set -euo pipefail

source_root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
cleanup() {
  [[ -z "${holder:-}" ]] || kill "$holder" 2>/dev/null || true
  rm -rf "${work:?}"
}
trap cleanup EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test
export GIT_COMMITTER_EMAIL=test@example.invalid

failures=0
pass() { printf 'ok   %s\n' "$1"; }
flunk() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}
check() {
  [[ -z "${TEST_DEBUG:-}" ]] || echo "-- status=${status:-} status_bg=${status_bg:-}" >&2
  if eval "$2"; then pass "$1"; else flunk "$1"; fi
}

# --- the stand-in Mac ------------------------------------------------------------------------------------------------

mac="$work/mac"
bin="$work/bin"
mkdir -p "$mac" "$bin"
cat >"$bin/ssh" <<'EOF'
#!/usr/bin/env bash
while (($#)); do
  case "$1" in
    -o) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
shift
cd "$FAKE_MAC_HOME"
export HOME="$FAKE_MAC_HOME"
unset XDG_CACHE_HOME
exec bash -c "$*"
EOF
cat >"$bin/lockf" <<'EOF'
#!/usr/bin/env bash
wait=()
while (($#)); do
  case "$1" in
    -[ksn]*) shift ;;
    -t)
      if (($2 == 0)); then wait=(-n); else wait=(-w "$2"); fi
      shift 2
      ;;
    *) break ;;
  esac
done
lock="$1"
shift
flock -E 75 ${wait[@]+"${wait[@]}"} "$lock" "$@"
EOF
chmod +x "$bin/ssh" "$bin/lockf"
export PATH="$bin:$PATH" FAKE_MAC_HOME="$mac"
mac_lock="$mac/.cache/ngterm/build-ios.flock"

# --- a checkout to copy ----------------------------------------------------------------------------------------------

repo="$work/repo"
mkdir -p "$repo/scripts/lib" "$repo/build" "$repo/src"
cp "$source_root/scripts/ios-mac-run.sh" "$repo/scripts/"
cp "$source_root/scripts/lib/native.sh" "$repo/scripts/lib/"
cp "$source_root/build/pins.env" "$repo/build/"
printf 'ignored/\ntarget/\n' >"$repo/.gitignore"
printf 'one\n' >"$repo/src/a.txt"
printf 'gone soon\n' >"$repo/src/b.txt"
printf '#!/bin/sh\n' >"$repo/src/tool.sh"
chmod +x "$repo/src/tool.sh"
ln -s a.txt "$repo/src/link"
git -C "$repo" init --quiet -b team/test-runner
git -C "$repo" add -A
git -C "$repo" commit --quiet -m test
printf 'untracked\n' >"$repo/src/new.txt"
mkdir -p "$repo/ignored"
printf 'ignored\n' >"$repo/ignored/x.txt"
runner="$repo/scripts/ios-mac-run.sh"
scratch="$mac/.cache/ngterm/run/team-test-runner"
logs="$repo/target/ios-mac-run/team-test-runner"
out="$work/out"
err="$work/err"

# Runs the runner, its output in $out and $err; sets $status.
run() {
  status=0
  timeout 120 "$@" >"$out" 2>"$err" || status=$?
  [[ -z "${TEST_DEBUG:-}" ]] || { echo "== $* -> $status"; cat "$out" "$err"; } >&2
}

# --- arguments -------------------------------------------------------------------------------------------------------

run "$runner"
check "no operation is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run
check "run with no command is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --timeout soon -- true
check "a time limit that is not a number is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --fetch ../x -- true
check "a fetch path out of the tree is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --name 'a/b' -- true
check "a name with a slash is refused" '((status == 1)) && grep -q "is not 1 to 48" "$err"'
run env -u IOS_MAC_HOST "$runner" run -- true
check "no Mac configured is refused with what to set" '((status == 1)) && grep -q "IOS_MAC_HOST" "$err"'
check "the local config file is git-ignored in the repository" \
  'git -C "$source_root" check-ignore -q scripts/ios-mac-run.local.env'
printf 'IOS_MAC_HOST=fake-mac\n' >"$repo/scripts/ios-mac-run.local.env"
printf 'scripts/ios-mac-run.local.env\n' >>"$repo/.git/info/exclude"

# --- a run: copy, lock, environment, exit status, logs ---------------------------------------------------------------

run env -u IOS_MAC_HOST "$runner" run -- bash -c \
  'cat src/a.txt; echo "lock=$BUILD_IOS_LOCK_HELD stamp=$BUILD_IOS_TREE_STAMP cache=$BUILD_IOS_CORE_CACHE"; exit 3'
check "the command's exit status is the runner's" '((status == 3))'
check "the command ran in the copied tree" 'grep -qx one "$out"'
check "the command holds the build lock (BUILD_IOS_LOCK_HELD)" 'grep -qF "lock=$mac_lock " "$out"'
check "the command gets the tree's checksum and the core cache" \
  'grep -qE "stamp=[0-9a-f]{64} cache=$mac/.cache/ngterm/ios-core$" "$out"'
check "the tree on the Mac has the untracked file" '[[ -f "$scratch/tree/src/new.txt" ]]'
check "the tree on the Mac has no ignored file" '[[ ! -e "$scratch/tree/ignored/x.txt" ]]'
check "an executable stays executable and a link a link" \
  '[[ -x "$scratch/tree/src/tool.sh" && -L "$scratch/tree/src/link" ]]'
check "the log is copied back" 'grep -qx one "$logs/command.log" && grep -qx "exit: 3" "$logs/run.txt"'
check "the summary says the exit status" 'grep -q "exit 3; copy" "$err"'
stamp1="$(sed -n 's/.*stamp=\([0-9a-f]*\).*/\1/p' "$out")"

# --- the next run: removal, build output kept, drift -----------------------------------------------------------------

rm "$repo/src/b.txt"
mkdir -p "$scratch/tree/target"
printf 'built\n' >"$scratch/tree/target/out.o"
printf 'ONE\n' >"$scratch/tree/src/a.txt"
touch -r "$repo/src/a.txt" "$scratch/tree/src/a.txt"
run "$runner" run -- bash -c 'cat src/a.txt; echo "stamp=$BUILD_IOS_TREE_STAMP"'
check "a file that drifted on the Mac (same size and time) is copied again" \
  '((status == 0)) && grep -qx one "$out" && grep -q "differ on the Mac" "$err"'
check "a file the tree lost is removed on the Mac" '[[ ! -e "$scratch/tree/src/b.txt" ]]'
check "build output on the Mac stays" '[[ -f "$scratch/tree/target/out.o" ]]'
stamp2="$(sed -n 's/.*stamp=\([0-9a-f]*\).*/\1/p' "$out")"
check "the tree's checksum changes with the tree" '[[ -n "$stamp2" && "$stamp1" != "$stamp2" ]]'
run "$runner" run -- bash -c 'echo "stamp=$BUILD_IOS_TREE_STAMP"'
check "an unchanged tree keeps its checksum" 'grep -qx "stamp=$stamp2" "$out"'

# --- the time limit, the lock wait, one run per name, cancel ---------------------------------------------------------

run "$runner" run --timeout 2 -- bash -c 'echo $$ >sleeper.pid; sleep 60 & wait'
sleeper="$(cat "$scratch/tree/sleeper.pid" 2>/dev/null || echo 0)"
check "a command past its time limit gives 124" '((status == 124)) && grep -q "stopping it" "$logs/command.log"'
check "the stopped command's process group is gone" '! kill -0 "$sleeper" 2>/dev/null && ! pgrep -x -P "$sleeper" sleep >/dev/null'

mkdir -p "$(dirname "$mac_lock")"
flock "$mac_lock" sleep 30 &
holder=$!
sleep 1
run "$runner" run --queue-timeout 2 -- true
check "a lock wait past its limit gives 75" '((status == 75)) && grep -q "not free within 2 s" "$logs/command.log"'
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true
holder=""

"$runner" run -- sleep 30 >"$work/bg.out" 2>"$work/bg.err" &
background=$!
for _ in $(seq 1 50); do
  [[ ! -s "$scratch/run/cmdpid" ]] || break
  sleep 0.2
done
run "$runner" run -- true
check "a second run of the same name is refused while one runs" '((status == 1)) && grep -q "still going" "$err"'
run "$runner" clean
check "clean refuses while a run is going" '((status == 3)) && [[ -d "$scratch" ]]'
run "$runner" cancel
status_bg=0
wait "$background" || status_bg=$?
check "cancel stops the run, whose runner gives 143" '((status == 0 && status_bg == 143))' || cat "$work/bg.err" >&2
run "$runner" wait
check "wait on an ended run gives its exit status at once" '((status == 143))'
run "$runner" run --clean -- true
check "--clean removes the scratch after the run" '((status == 0)) && [[ ! -e "$scratch" ]]'
run "$runner" list
check "list runs" '((status == 0)) && grep -q "^free: " "$out"'

# --- the iOS build lock under an outer lockf (lib/native.sh) --------------------------------------------------------

export XDG_CACHE_HOME="$work/cache"
lock="$XDG_CACHE_HOME/ngterm/build-ios.flock"
mkdir -p "$(dirname "$lock")"
native="$source_root/scripts/lib/native.sh"
probe="$work/probe.sh"
printf 'source %q\ntake_ios_build_lock probe "$0"\necho "held=$BUILD_IOS_LOCK_HELD"\n' "$native" >"$probe"
run lockf -k "$lock" bash "$probe"
check "under an outer lockf of the build lock a script goes on, holding it" \
  '((status == 0)) && grep -qx "held=$lock" "$out" && grep -q "outer lockf holds" "$err"'
run lockf -k "$work/other.flock" bash -c "source $(printf %q "$native"); lock_held_by_ancestor $(printf %q "$lock")"
check "another lock file's lockf is no holder" '((status == 1))' || cat "$err" >&2
run env -u BUILD_IOS_LOCK_HELD bash "$probe"
check "with no holder the script takes the lock itself" '((status == 0)) && grep -qx "held=$lock" "$out"'
unset XDG_CACHE_HOME

# --- the core cache (lib/ios-core-cache.sh) --------------------------------------------------------------------------

ws="$work/ws"
for crate in core dep other devonly; do
  mkdir -p "$ws/crates/$crate/src"
  printf 'pub fn f() {}\n' >"$ws/crates/$crate/src/lib.rs"
done
printf '[workspace]\nresolver = "3"\nmembers = ["crates/*"]\n' >"$ws/Cargo.toml"
for crate in dep other devonly; do
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2024"\n' "$crate" >"$ws/crates/$crate/Cargo.toml"
done
printf '[package]\nname = "core"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\ndep = { path = "../dep" }
[dev-dependencies]\ndevonly = { path = "../devonly" }\n' >"$ws/crates/core/Cargo.toml"
(cd "$ws" && cargo generate-lockfile --offline --quiet)
key() {
  # shellcheck source=lib/native.sh
  # shellcheck source=lib/ios-core-cache.sh
  bash -c 'source "$1/scripts/lib/native.sh"; source "$1/scripts/lib/ios-core-cache.sh"; ios_core_cache_key "$2" core "$3" "rustc 1"' \
    key "$source_root" "$ws" "${1:-release}"
}
key1="$(key)"
check "a core key is made" '[[ "$key1" =~ ^[0-9a-f]{32}$ ]]'
printf '// more\n' >>"$ws/crates/other/src/lib.rs"
printf '// more\n' >>"$ws/crates/devonly/src/lib.rs"
check "a change outside of the core's dependencies keeps the key" '[[ "$(key)" == "$key1" ]]'
check "another profile is another key" '[[ "$(key debug)" != "$key1" ]]'
printf '// more\n' >>"$ws/crates/dep/src/lib.rs"
check "a change in a dependency of the core is another key" '[[ "$(key)" != "$key1" ]]'

cache="$work/cache-dir"
core_out="$work/core"
mkdir -p "$core_out/NgtermCoreFFI.xcframework"
head -c 614400 /dev/zero >"$core_out/NgtermCoreFFI.xcframework/lib.a"
printf 'swift\n' >"$core_out/NgtermCoreBindings.swift"
cache_call() {
  bash -c 'source "$1/scripts/lib/native.sh"; source "$1/scripts/lib/ios-core-cache.sh"; shift; "$@"' \
    cache "$source_root" "$@"
}
cache_call ios_core_cache_store "$cache" aaaa "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  1 0 2>"$err"
check "a stored core is found" '[[ "$(cache_call ios_core_cache_entry "$cache" aaaa)" == "$cache/aaaa" ]]'
check "a missing core is not found" '! cache_call ios_core_cache_entry "$cache" bbbb >/dev/null'
cache_call ios_core_cache_store "$cache" bbbb "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  1 0 2>"$err"
check "the cache stays under its size limit: the oldest core goes" \
  '[[ ! -e "$cache/aaaa" && -f "$cache/bbbb/.complete" ]] && grep -q "removed the cached core aaaa" "$err"'
cache_call ios_core_cache_store "$cache" cccc "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  100 99999999 2>"$err"
check "nothing is stored when the disk would have too little free" \
  '[[ ! -e "$cache/cccc" ]] && grep -q "would stay free" "$err"'
mkdir -p "$cache/.tmp-dddd" "$cache/eeee"
cache_call ios_core_cache_prune "$cache" 100000
check "a prune removes stopped stores and incomplete entries" \
  '[[ ! -e "$cache/.tmp-dddd" && ! -e "$cache/eeee" && -d "$cache/bbbb" ]]'

if ((failures)); then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all checks passed"
