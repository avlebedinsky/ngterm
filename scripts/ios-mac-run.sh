#!/usr/bin/env bash
# Runs one command for the iOS app on a shared Mac, from a Linux host, in a scratch copy of this checkout:
#
# 1. copy: the whole working tree (tracked files and untracked ones git doesn't ignore, uncommitted changes included)
#    goes to the Mac's scratch directory of the run's name, ~/.cache/ngterm/run/<name>/tree, with rsync. Files the
#    tree no longer has are removed there; build output (target/, mobile/ios/build/, Generated/) stays, so a second
#    run of the same name builds only what changed. Then a checksum of every file (content, executable bit, link
#    target) is taken on both sides: a file that differs is copied again by content, and a tree that still differs
#    stops the run, so the Mac never tests another tree than this one. A copy that did not end leaves no file behind:
#    the next one removes what it brought that the tree lacks. The launch claims the scratch from the copy to the
#    collection of the results, so a second launch of the name is refused (the claim of a launch that died holds
#    for an hour after its last call, or until `cancel`);
# 2. queue: the command waits for the iOS build lock (~/.cache/ngterm/build-ios.flock on the Mac, one xcodebuild at a
#    time) for at most --queue-timeout seconds. It runs under that hold with BUILD_IOS_LOCK_HELD set, so
#    scripts/build-ios.sh and scripts/test-ios-e2e.sh take no lock of their own: never wrap them in a lockf yourself;
# 3. run: the command runs in the tree, in a process group of its own, for at most --timeout seconds; then the whole
#    group gets TERM, and KILL 30 s later. It gets BUILD_IOS_TREE_STAMP (the tree's checksum: build-ios.sh then builds
#    a test plan once per tree and only tests it on the next simulator and in repeat runs) and BUILD_IOS_CORE_CACHE
#    (~/.cache/ngterm/ios-core on the Mac: a core with the same inputs is built once for every worker),
#    BUILD_IOS_BUILD_NUMBER (this checkout's commit count, for the archive: the copy has no Git history) and
#    ~/.cargo/bin on PATH;
# 4. wait: the run is detached from ssh, so a dropped connection or a killed runner (SIGKILL) doesn't stop it, and
#    `wait` waits for it again. A runner that gets INT (Ctrl-C), TERM or HUP cancels the run and exits with 130, so
#    the Mac is not busy for nobody. The runner waits in ssh calls of at most two minutes each, prints the command's
#    output as it comes, and gives up at the run's limits (`wait` reads them from the Mac) plus five minutes from the
#    run's start in the queue; a run that ends with no exit status is a failure, never a wait that goes on;
# 5. results: the command's log (and each --fetch path of the tree) is copied into --logs, and the runner exits with
#    the command's exit status, or: 124 when it ran out of time, 75 when the lock wait ran out, 143 when the run was
#    cancelled, 130 when the runner stopped on a signal (and cancelled the run), 1 when the runner itself failed (its
#    messages start with "ios-mac-run.sh:"), 2 for a usage error. cancel, clean and list give the Mac's status: 3
#    when a run or another launch holds the scratch, 255 when ssh failed;
# 6. clean: --clean (or `clean`) removes the scratch directory. Remove it when the task's Mac work is done: a scratch
#    with its build output takes several GB.
#
# The Mac is the ssh destination IOS_MAC_HOST, from the environment or from scripts/ios-mac-run.local.env
# (git-ignored; one IOS_MAC_HOST=<destination> line), in this checkout or in the main checkout of its repository. The
# runner refuses to start when the Mac has less than IOS_MAC_MIN_FREE_GB (default 8) GB free.
#
# Usage: scripts/ios-mac-run.sh run [--name NAME] [--timeout SECONDS] [--queue-timeout SECONDS] [--logs DIR]
#                                   [--env NAME=VALUE]... [--fetch PATH]... [--clean] -- COMMAND [ARG...]
#        scripts/ios-mac-run.sh wait [--name NAME] [--logs DIR] [--fetch PATH]... [--clean]
#        scripts/ios-mac-run.sh cancel|clean [--name NAME]
#        scripts/ios-mac-run.sh list
#   --name           the scratch directory's name (default: the branch, / as -); one run per name at a time
#   --timeout        the command's time limit once it holds the lock (default 7200; wait takes the run's own)
#   --queue-timeout  the time limit of the wait for the lock (default 10800; wait takes the run's own)
#   --logs           where the log and the fetched paths go (default: target/ios-mac-run/<name>/ of this checkout)
#   --env            a variable for the command (repeatable)
#   --fetch          a path of the tree to copy back after the run, as mobile/ios/build/UI.xcresult (repeatable)
#   --clean          remove the scratch directory after the run
#   wait             wait again for the last run of that name; cancel stops it; clean removes its scratch
#   list             the Mac's scratch directories, their size and state, the core cache and the free disk
# A Mac's scratch is the operator's own disk: run `list` and remove what is yours.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

SELF=ios-mac-run.sh
usage() {
  echo "usage: scripts/ios-mac-run.sh run [--name NAME] [--timeout SECONDS] [--queue-timeout SECONDS] [--logs DIR]
                                  [--env NAME=VALUE]... [--fetch PATH]... [--clean] -- COMMAND [ARG...]
       scripts/ios-mac-run.sh wait [--name NAME] [--logs DIR] [--fetch PATH]... [--clean]
       scripts/ios-mac-run.sh cancel|clean [--name NAME]
       scripts/ios-mac-run.sh list" >&2
  exit 2
}
fail() {
  echo "$SELF: $*" >&2
  exit 1
}

# --- arguments -------------------------------------------------------------------------------------------------------

(($#)) || usage
OP="$1"
shift
case "$OP" in
  run | wait | cancel | clean | list) ;;
  *) usage ;;
esac
NAME=""
TIMEOUT=7200
QUEUE_TIMEOUT=10800
LOGS=""
CLEAN=0
ENVS=()
FETCH=()
COMMAND=()
while (($#)); do
  case "$1" in
    --name | --timeout | --queue-timeout | --logs | --env | --fetch)
      (($# >= 2)) || usage
      case "$1" in
        --name) NAME="$2" ;;
        --timeout | --queue-timeout)
          [[ "$2" =~ ^[1-9][0-9]*$ ]] || usage
          if [[ "$1" == --timeout ]]; then TIMEOUT="$2"; else QUEUE_TIMEOUT="$2"; fi
          ;;
        --logs) LOGS="$2" ;;
        --env)
          [[ "$2" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || usage
          ENVS+=("$2")
          ;;
        --fetch)
          [[ "$2" != /* && "/$2/" != */../* && -n "$2" ]] || usage
          FETCH+=("$2")
          ;;
      esac
      shift 2
      ;;
    --clean)
      CLEAN=1
      shift
      ;;
    --)
      shift
      COMMAND=("$@")
      break
      ;;
    *) usage ;;
  esac
done
case "$OP" in
  run) ((${#COMMAND[@]})) || usage ;;
  *) ((${#COMMAND[@]} == 0 && ${#ENVS[@]} == 0)) || usage ;;
esac
[[ "$OP" == run || "$OP" == wait ]] || ((CLEAN == 0 && ${#FETCH[@]} == 0)) || usage
[[ "$OP" != list || -z "$NAME" ]] || usage

if [[ -z "$NAME" && "$OP" != list ]]; then
  NAME="$(git -C "$NATIVE_ROOT" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
  NAME="${NAME//\//-}"
  [[ -n "$NAME" ]] || fail "HEAD is detached: give the run a --name"
fi
if [[ "$OP" != list ]] && [[ ! "$NAME" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$ ]]; then
  fail "the name '$NAME' is not 1 to 48 letters, digits, '.', '_' or '-' (starting with a letter or digit)"
fi
[[ -n "$LOGS" ]] || LOGS="$NATIVE_ROOT/target/ios-mac-run/$NAME"

# The Mac: IOS_MAC_HOST, or the git-ignored local file of this checkout or of the repository's main checkout.
local_env_value() {
  local file common
  common="$(git -C "$NATIVE_ROOT" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  for file in "$NATIVE_ROOT/scripts/ios-mac-run.local.env" ${common:+"$(dirname "$common")/scripts/ios-mac-run.local.env"}; do
    [[ -f "$file" ]] || continue
    sed -n "s/^$1=//p" "$file" | tail -n1
    return 0
  done
}
HOST="${IOS_MAC_HOST:-$(local_env_value IOS_MAC_HOST)}"
[[ -n "$HOST" ]] || fail "no Mac: set IOS_MAC_HOST, or write IOS_MAC_HOST=<ssh destination> into scripts/ios-mac-run.local.env"
[[ "$HOST" =~ ^[A-Za-z0-9_.@:-]+$ ]] || fail "IOS_MAC_HOST '$HOST' is not an ssh destination"
MIN_FREE_GB="${IOS_MAC_MIN_FREE_GB:-8}"
[[ "$MIN_FREE_GB" =~ ^[0-9]+$ ]] || fail "IOS_MAC_MIN_FREE_GB '$MIN_FREE_GB' is not a number of GB"

# The scratch directory, relative to the Mac's home (rsync's paths are) and as the remote shell sees it.
RELATIVE=".cache/ngterm/run/$NAME"

# --- the Mac's side --------------------------------------------------------------------------------------------------

# Every Mac call runs this library in `bash -s` (the login shell may be another), after the variables of
# remote_call. Portable to stock macOS's bash 3.2.
# shellcheck disable=SC2016 # the Mac's bash expands it
REMOTE_LIB='
SCRATCH="$HOME/$RELATIVE"
RUN="$SCRATCH/run"
LOCK="${XDG_CACHE_HOME:-$HOME/.cache}/ngterm/build-ios.flock"
CORE_CACHE="$HOME/.cache/ngterm/ios-core"

CLAIM="$SCRATCH/claim"

# Whether the run lives: it has no exit status yet and its job is there (a stored pid of an ended run is no proof).
alive() { [[ ! -f "$RUN/exit" && -s "$RUN/pid" ]] && kill -0 "$(cat "$RUN/pid")" 2>/dev/null; }

# The claim of the scratch, "<id> <epoch>": the launch of that id copies into it, runs in it or collects from it. It
# holds while its run lives, else for an hour after its last renewal (prepare, verify, start and each poll renew it),
# so a launch that died frees the scratch after an hour; cancel frees it at once.
claim_owner() {
  if [[ -s "$CLAIM" ]]; then awk "{ print \$1; exit }" "$CLAIM"; fi
}
claim_held() {
  local renewed
  [[ -s "$CLAIM" ]] || return 1
  if alive; then return 0; fi
  renewed="$(awk "{ print \$2; exit }" "$CLAIM")"
  [[ "$renewed" =~ ^[0-9]+$ ]] && (($(date +%s) - renewed < 3600))
}
claim_renew() {
  printf "%s %s\n" "$1" "$(date +%s)" >"$CLAIM.tmp"
  mv "$CLAIM.tmp" "$CLAIM"
}
# Exits 3 unless this launch (RUN_ID) holds the claim.
claim_check() {
  if [[ -z "$RUN_ID" || "$(claim_owner)" != "$RUN_ID" ]]; then
    echo "another launch ($(claim_owner)) took the scratch $RELATIVE over" >&2
    exit 3
  fi
}

# The manifest of the NUL-separated paths of file $2 below directory $1: "<sha256> <kind> <path>" per line, kind f
# (a file), x (an executable file), l (a symbolic link, hashed by its target) or - (missing).
manifest() {
  python3 -c "
import hashlib, os, stat, sys
root = sys.argv[1].encode()
with open(sys.argv[2], \"rb\") as listing:
    paths = [path for path in listing.read().split(b\"\\0\") if path]
for path in paths:
    full = os.path.join(root, path)
    try:
        info = os.lstat(full)
    except FileNotFoundError:
        kind, digest = \"-\", \"-\"
    else:
        if stat.S_ISLNK(info.st_mode):
            kind, digest = \"l\", hashlib.sha256(b\"link:\" + os.readlink(full)).hexdigest()
        elif stat.S_ISREG(info.st_mode):
            content = hashlib.sha256()
            with open(full, \"rb\") as handle:
                for chunk in iter(lambda: handle.read(1 << 20), b\"\"):
                    content.update(chunk)
            kind, digest = (\"x\" if info.st_mode & 0o111 else \"f\"), content.hexdigest()
        else:
            kind, digest = \"?\", \"-\"
    sys.stdout.write(digest + \" \" + kind + \" \" + path.decode(\"utf-8\", \"surrogateescape\") + \"\\n\")
" "$1" "$2"
}

op_prepare() {
  local free
  free="$(df -Pk "$HOME" | awk "NR == 2 { print int(\$4 / 1048576) }")"
  if alive; then
    echo "a run of $RELATIVE is still going (pid $(cat "$RUN/pid")): wait for it or cancel it" >&2
    exit 3
  fi
  if ((free < MIN_FREE_GB)); then
    echo "the Mac has $free GB free, less than $MIN_FREE_GB GB: free some (scripts/ios-mac-run.sh list) first" >&2
    exit 4
  fi
  mkdir -p "$SCRATCH/tree"
  if ! (set -C && printf "%s %s\n" "$RUN_ID" "$(date +%s)" >"$CLAIM") 2>/dev/null; then
    if [[ "$(claim_owner)" != "$RUN_ID" ]] && claim_held; then
      echo "another launch ($(claim_owner)) copies into $RELATIVE or collects from it: wait for it, or cancel it" >&2
      exit 3
    fi
    claim_renew "$RUN_ID"
  fi
  # A copy that did not end can have brought paths that the tree no longer has: they join the list of the last copy,
  # so that the check removes them.
  if [[ -f "$SCRATCH/incoming/files" ]]; then
    python3 -c "
import os, sys
scratch = sys.argv[1]
paths = set()
for name in (\"files\", \"incoming/files\"):
    try:
        with open(os.path.join(scratch, name), \"rb\") as listing:
            paths |= {path for path in listing.read().split(b\"\\0\") if path}
    except FileNotFoundError:
        pass
with open(os.path.join(scratch, \"files.tmp\"), \"wb\") as merged:
    merged.write(b\"\".join(path + b\"\\0\" for path in sorted(paths)))
os.replace(os.path.join(scratch, \"files.tmp\"), os.path.join(scratch, \"files\"))
" "$SCRATCH"
  fi
  rm -rf "${SCRATCH:?}/incoming"
  mkdir -p "$SCRATCH/incoming"
  echo "$free"
}

# Removes the files the last copy brought that the new tree lacks, then prints the paths whose checksum differs from
# the one made on the host; the new manifest becomes the current one when none does.
op_verify() {
  claim_check
  claim_renew "$RUN_ID"
  python3 -c "
import os, sys
scratch = sys.argv[1]
def paths(name):
    try:
        with open(os.path.join(scratch, name), \"rb\") as listing:
            return {path for path in listing.read().split(b\"\\0\") if path}
    except FileNotFoundError:
        return set()
tree = os.path.join(scratch, \"tree\").encode()
for path in sorted(paths(\"files\") - paths(\"incoming/files\")):
    if path.startswith(b\"/\") or b\"..\" in path.split(b\"/\"):
        continue
    full = os.path.join(tree, path)
    if os.path.islink(full) or os.path.isfile(full):
        os.remove(full)
" "$SCRATCH"
  manifest "$SCRATCH/tree" "$SCRATCH/incoming/files" >"$SCRATCH/incoming/manifest.mac"
  if cmp -s "$SCRATCH/incoming/manifest" "$SCRATCH/incoming/manifest.mac"; then
    mv "$SCRATCH/incoming/files" "$SCRATCH/files"
    mv "$SCRATCH/incoming/manifest" "$SCRATCH/manifest"
    rm -rf "${SCRATCH:?}/incoming"
    return 0
  fi
  { diff "$SCRATCH/incoming/manifest" "$SCRATCH/incoming/manifest.mac" || true; } | sed -n "s/^< [^ ]* [^ ]* //p"
}

op_start() {
  claim_check
  if [[ -s "$RUN/id" && "$(cat "$RUN/id")" == "$RUN_ID" && -s "$RUN/pid" ]]; then
    cat "$RUN/pid" # a retry of this call after a lost connection: the run started then
    return 0
  fi
  if alive; then
    echo "a run of $RELATIVE is still going (pid $(cat "$RUN/pid")): wait for it or cancel it" >&2
    exit 3
  fi
  rm -rf "${RUN:?}"
  mkdir -p "$RUN"
  echo "$TIMEOUT $QUEUE_TIMEOUT" >"$RUN/limits"
  printf "%s\n" "$COMMAND" >"$RUN/command"
  printf "%s\n" "$ENVS" >"$RUN/env"
  cat >"$RUN/command.sh" <<EOF
set -u
run=$(printf %q "$RUN")
date +%s >"\$run/started"
export PATH="\$HOME/.cargo/bin:\$PATH" BUILD_IOS_LOCK_HELD=$(printf %q "$LOCK")
export BUILD_IOS_TREE_STAMP=$(printf %q "$STAMP") BUILD_IOS_CORE_CACHE=$(printf %q "$CORE_CACHE")
eval "\$(cat "\$run/env")"
eval "set -- \$(cat "\$run/command")"
! command -v caffeinate >/dev/null || set -- caffeinate -is "\$@"
cd $(printf %q "$SCRATCH/tree")
perl -e "setpgrp(0, 0); exec @ARGV or die qq(cannot run \\\$ARGV[0]: \\\$!\\\\n)" -- "\$@" </dev/null >>"\$run/command.log" 2>&1 &
pid=\$!
echo "\$pid" >"\$run/cmdpid"
elapsed=0 stopped=0
while kill -0 "\$pid" 2>/dev/null; do
  if ((elapsed >= $TIMEOUT)); then
    echo "$SELF: the command still runs after ${TIMEOUT} s: stopping it" >>"\$run/command.log"
    kill -TERM -- "-\$pid" 2>/dev/null || true
    grace=0
    while ((grace < 30)) && kill -0 -- "-\$pid" 2>/dev/null; do
      sleep 1
      grace=\$((grace + 1))
    done
    kill -KILL -- "-\$pid" 2>/dev/null || true
    stopped=1
    break
  fi
  sleep 1
  elapsed=\$((elapsed + 1))
done
status=0
wait "\$pid" || status=\$?
((stopped == 0)) || status=124
kill -TERM -- "-\$pid" 2>/dev/null || true
echo "\$status" >"\$run/status"
EOF
  cat >"$RUN/job.sh" <<EOF
set -u
run=$(printf %q "$RUN")
date +%s >"\$run/queued"
status=0
mkdir -p $(printf %q "$(dirname "$LOCK")")
lockf -k -t $QUEUE_TIMEOUT $(printf %q "$LOCK") /bin/bash "\$run/command.sh" || status=\$?
if [[ -s "\$run/status" ]]; then
  status=\$(cat "\$run/status")
elif ((status == 75)); then
  echo "$SELF: the build lock was not free within ${QUEUE_TIMEOUT} s" >>"\$run/command.log"
fi
date +%s >"\$run/ended"
echo "\$status" >"\$run/exit.tmp"
mv "\$run/exit.tmp" "\$run/exit"
EOF
  : >"$RUN/command.log"
  nohup perl -e "setpgrp(0, 0); exec @ARGV or die" -- /bin/bash "$RUN/job.sh" </dev/null >"$RUN/job.log" 2>&1 &
  echo "$!" >"$RUN/pid"
  echo "$RUN_ID" >"$RUN/id"
  claim_renew "$RUN_ID"
  echo "$!"
}

# One line of state, then the log from byte OFFSET on: waits up to 30 s for the run to end or the log to grow.
op_poll() {
  local state size=0 waited=0 code="" value name times="" id="$RUN_ID" limits="- -"
  [[ -n "$id" ]] || id="$(cat "$RUN/id" 2>/dev/null || true)"
  if [[ -n "$id" && "$(claim_owner)" == "$id" ]]; then claim_renew "$id"; fi
  while :; do
    [[ ! -f "$RUN/command.log" ]] || size="$(wc -c <"$RUN/command.log" | tr -d " ")"
    if [[ -f "$RUN/exit" ]]; then
      state=done code="$(cat "$RUN/exit")"
    elif ! alive && [[ -f "$RUN/cancelled" ]]; then
      state=done code=143
    elif alive; then
      if [[ -f "$RUN/started" ]]; then state=running; else state=queued; fi
    elif [[ -d "$RUN" ]]; then
      state=lost
    else
      state=none
    fi
    if [[ "$state" != queued && "$state" != running ]] || ((size > OFFSET || waited >= 30)); then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  for name in queued started ended; do
    value=""
    [[ ! -f "$RUN/$name" ]] || value="$(cat "$RUN/$name")"
    times="$times $name=${value:--}"
  done
  [[ ! -s "$RUN/limits" ]] || limits="$(cat "$RUN/limits")"
  echo "state=$state exit=${code:--} size=$size$times now=$(date +%s) limit=${limits% *} queue_limit=${limits#* }"
  if ((size > OFFSET)); then
    tail -c +$((OFFSET + 1)) "$RUN/command.log" | head -c $((size - OFFSET))
  fi
}

op_cancel() {
  local grace=0 pid
  [[ ! -d "$RUN" || -f "$RUN/exit" ]] || : >"$RUN/cancelled"
  if [[ -s "$RUN/cmdpid" && ! -f "$RUN/status" && ! -f "$RUN/exit" ]]; then
    pid="$(cat "$RUN/cmdpid")"
    kill -TERM -- "-$pid" 2>/dev/null || true
    while ((grace < 30)) && kill -0 -- "-$pid" 2>/dev/null; do
      sleep 1
      grace=$((grace + 1))
    done
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
  if alive; then
    pid="$(cat "$RUN/pid")"
    kill -TERM -- "-$pid" 2>/dev/null || true
    sleep 2
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
  if [[ -d "$RUN" && ! -f "$RUN/exit" ]]; then
    echo 143 >"$RUN/exit"
    echo "$SELF: the run was cancelled" >>"$RUN/command.log"
  fi
  rm -f "$CLAIM"
}

# Frees the claim of this launch, or (from wait, with no RUN_ID) the claim of the run of the scratch.
op_release() {
  local id="$RUN_ID"
  [[ -n "$id" ]] || id="$(cat "$RUN/id" 2>/dev/null || true)"
  if [[ -n "$id" && "$(claim_owner)" == "$id" ]]; then rm -f "$CLAIM"; fi
}

op_clean() {
  if alive; then
    echo "a run of $RELATIVE is still going (pid $(cat "$RUN/pid")): cancel it first" >&2
    exit 3
  fi
  if claim_held; then
    echo "a launch ($(claim_owner)) copies into $RELATIVE or collects from it: cancel it first" >&2
    exit 3
  fi
  rm -rf "${SCRATCH:?}"
}

op_list() {
  local dir state
  for dir in "$HOME/.cache/ngterm/run"/*; do
    [[ -d "$dir" ]] || continue
    RUN="$dir/run"
    if alive; then state=running; elif [[ -f "$RUN/exit" ]]; then state="exit $(cat "$RUN/exit")"; else state=idle; fi
    printf "%-48s %8s  %s\n" "${dir##*/}" "$(du -sh "$dir" 2>/dev/null | cut -f1)" "$state"
  done
  [[ ! -d "$CORE_CACHE" ]] || printf "%-48s %8s\n" "(core cache)" "$(du -sh "$CORE_CACHE" | cut -f1)"
  df -h "$HOME" | awk "NR == 2 { print \"free: \" \$4 }"
}
'

SSH_OPTIONS=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4)

# Runs the Mac's operation $1 in at most $2 seconds, with the variables NAME=VALUE $3... (shell-quoted on the way).
# A connection that fails (ssh's 255: an asleep Mac answers after a while) is tried twice more.
remote_call() {
  local op="$1" limit="$2" try status script assignment
  shift 2
  script="set -euo pipefail"$'\n'
  for assignment in "RELATIVE=$RELATIVE" "MIN_FREE_GB=$MIN_FREE_GB" "SELF=$SELF" "RUN_ID=$RUN_ID" "$@"; do
    script+="${assignment%%=*}=$(printf %q "${assignment#*=}")"$'\n'
  done
  script+="$REMOTE_LIB"$'\n'"op_$op"$'\n'
  for try in 1 2 3; do
    status=0
    timeout --kill-after=10 "$limit" ssh "${SSH_OPTIONS[@]}" "$HOST" bash -s <<<"$script" || status=$?
    ((status == 255 && try < 3)) || return "$status"
    echo "$SELF: no ssh connection to the Mac (try $try of 3)" >&2
    sleep 10
  done
}

rsync_to_mac() {
  timeout --kill-after=10 1800 rsync -e "ssh ${SSH_OPTIONS[*]}" "$@"
}

# --- the host's side -------------------------------------------------------------------------------------------------

# The host's manifest, as the Mac's manifest() makes it, of the NUL-separated paths of file $1 below the checkout.
host_manifest() {
  local script
  script="$(sed -n '/^manifest() {$/,/^}$/p' <<<"$REMOTE_LIB")"
  bash -c "$script"$'\n''manifest "$@"' manifest "$NATIVE_ROOT" "$1"
}

# The working tree's paths, NUL-separated and sorted: what git tracks and what it doesn't ignore, minus what is gone.
tree_paths() {
  git -C "$NATIVE_ROOT" ls-files -z --cached --others --exclude-standard |
    python3 -c "
import os, sys
root = sys.argv[1].encode()
paths = sorted({path for path in sys.stdin.buffer.read().split(b'\0') if path and os.path.lexists(os.path.join(root, path))})
sys.stdout.buffer.write(b''.join(path + b'\0' for path in paths))
" "$NATIVE_ROOT"
}

# This launch's id: the Mac's claim of the scratch (prepare to the collection of the results) and its run carry it.
RUN_ID=""
[[ "$OP" != run ]] || RUN_ID="$(date +%s)-$$-$RANDOM"
WORK="$(mktemp -d)"
CLAIMED=0
STARTED=0
# A launch that stops before its run starts frees its claim of the scratch at once.
cleanup() {
  if ((CLAIMED && !STARTED)); then
    remote_call release 60 || echo "$SELF: couldn't free the claim of the Mac's scratch $RELATIVE" >&2
  fi
  rm -rf "${WORK:?}"
}
trap cleanup EXIT
on_signal() {
  if ((STARTED)); then
    echo "$SELF: stopped: cancelling the run on the Mac" >&2
    remote_call cancel 120 || true
  fi
  exit 130
}
trap on_signal INT TERM HUP

# Copies the tree to the Mac and checks it there; prints the tree's checksum.
copy_tree() {
  local free diff attempt
  free="$(remote_call prepare 120)" || fail "the Mac's scratch $RELATIVE is not ready (see above)"
  echo "$SELF: $NAME: copying the tree to the Mac ($free GB free there)" >&2
  tree_paths >"$WORK/files"
  host_manifest "$WORK/files" >"$WORK/manifest"
  rsync_to_mac -rlpt "$WORK/files" "$WORK/manifest" "$HOST:$RELATIVE/incoming/" || fail "rsync of the manifest failed"
  rsync_to_mac -rlpt --from0 --files-from="$WORK/files" "$NATIVE_ROOT/" "$HOST:$RELATIVE/tree/" ||
    fail "rsync of the tree failed"
  for attempt in 1 2; do
    diff="$(remote_call verify 600)" || fail "the tree's check on the Mac failed"
    [[ -n "$diff" ]] || {
      sha256_of <"$WORK/manifest"
      return 0
    }
    ((attempt == 1)) ||
      fail "the Mac's tree still differs from this one after a second copy: $(head -n 5 <<<"$diff" | tr '\n' ' ')"
    echo "$SELF: $(wc -l <<<"$diff") file(s) differ on the Mac after the copy: copying them by content" >&2
    tr '\n' '\0' <<<"$diff" >"$WORK/differ"
    rsync_to_mac -rlpt --checksum --from0 --files-from="$WORK/differ" "$NATIVE_ROOT/" "$HOST:$RELATIVE/tree/" ||
      fail "rsync of the differing files failed"
  done
}

# The value of the field $1 in the poll header $2, as digits (or - for none).
header_field() {
  sed -n "s/.* $1=\([0-9-]*\).*/\1/p" <<<"$2"
}

# Waits for the run until it ends or its deadline passes, printing its log; sets RESULT_* . The deadline is its limits
# (the Mac's record of them, else TIMEOUT and QUEUE_TIMEOUT) from its start in the queue (in the Mac's time, from the
# first poll on; before that from now) plus five minutes; it gives 75 when the run is still in the queue then, else 124.
wait_run() {
  local deadline offset=0 out="$WORK/poll" header state="" failures=0 announced="" now limit queue_limit end
  deadline=$(($(date +%s) + QUEUE_TIMEOUT + TIMEOUT + 300))
  while :; do
    if (($(date +%s) >= deadline)); then
      echo "$SELF: $NAME: the run is still going past its limits: cancelling it" >&2
      remote_call cancel 120 || true
      if [[ "$state" == queued ]]; then RESULT_EXIT=75; else RESULT_EXIT=124; fi
      return
    fi
    if ! remote_call poll 120 "OFFSET=$offset" >"$out"; then
      failures=$((failures + 1))
      ((failures < 5)) || fail "lost the Mac for five polls in a row; the run may go on: scripts/ios-mac-run.sh wait --name $NAME"
      sleep 10
      continue
    fi
    failures=0
    header="$(head -n1 "$out")"
    tail -n +2 "$out"
    offset="$(header_field size "$header")"
    state="$(sed -n 's/^state=\([a-z]*\).*/\1/p' <<<"$header")"
    RESULT_QUEUED="$(header_field queued "$header")"
    RESULT_STARTED="$(header_field started "$header")"
    RESULT_ENDED="$(header_field ended "$header")"
    now="$(header_field now "$header")"
    limit="$(header_field limit "$header")"
    queue_limit="$(header_field queue_limit "$header")"
    [[ "$limit" =~ ^[0-9]+$ ]] || limit="$TIMEOUT"
    [[ "$queue_limit" =~ ^[0-9]+$ ]] || queue_limit="$QUEUE_TIMEOUT"
    end=""
    if [[ "$RESULT_STARTED" =~ ^[0-9]+$ ]]; then
      end=$((RESULT_STARTED + limit + 300))
    elif [[ "$RESULT_QUEUED" =~ ^[0-9]+$ ]]; then
      end=$((RESULT_QUEUED + queue_limit + limit + 300))
    fi
    [[ -z "$end" || ! "$now" =~ ^[0-9]+$ ]] || deadline=$(($(date +%s) + end - now))
    case "$state" in
      queued)
        [[ -n "$announced" ]] || echo "$SELF: $NAME: waiting for the Mac's build lock" >&2
        announced=1
        ;;
      running) ;;
      done)
        RESULT_EXIT="$(header_field exit "$header")"
        [[ "$RESULT_EXIT" =~ ^[0-9]+$ ]] || fail "the run on the Mac gave no exit status: $header"
        return
        ;;
      lost) fail "the run on the Mac ended with no exit status (its job died): see $RELATIVE/run/job.log there" ;;
      none) fail "no run of $NAME on the Mac" ;;
      *) fail "the Mac's poll gave no state: $header" ;;
    esac
  done
}

# Copies the log and the --fetch paths into LOGS and prints the summary of the run that began (host time) at $1.
collect() {
  local began="$1" copied="$2" path queue="-" run="-"
  mkdir -p "$LOGS"
  rsync_to_mac -rlpt "$HOST:$RELATIVE/run/command.log" "$LOGS/command.log" ||
    echo "$SELF: couldn't copy the run's log from the Mac" >&2
  for path in ${FETCH[@]+"${FETCH[@]}"}; do
    rsync_to_mac -rlpt "$HOST:$RELATIVE/tree/$path" "$LOGS/" || echo "$SELF: couldn't fetch $path from the Mac" >&2
  done
  [[ "${RESULT_QUEUED:--}" == - || "${RESULT_STARTED:--}" == - ]] || queue="$((RESULT_STARTED - RESULT_QUEUED)) s"
  [[ "${RESULT_STARTED:--}" == - || "${RESULT_ENDED:--}" == - ]] || run="$((RESULT_ENDED - RESULT_STARTED)) s"
  {
    echo "name: $NAME"
    echo "tree: ${TREE:-(not copied by this call)}"
    echo "exit: $RESULT_EXIT"
    echo "copy: $copied"
    echo "queue: $queue"
    echo "run: $run"
    echo "turn: $(($(date +%s) - began)) s"
  } >"$LOGS/run.txt"
  echo "$SELF: $NAME: exit $RESULT_EXIT; copy $copied, queue $queue, run $run, turn $(($(date +%s) - began)) s;" \
    "log: $LOGS/command.log" >&2
}

# Frees the claim of the scratch, removes the scratch with --clean and exits with the run's exit status.
finish() {
  remote_call release 60 || echo "$SELF: couldn't free the claim of the Mac's scratch $RELATIVE" >&2
  CLAIMED=0
  if ((CLEAN)); then
    remote_call clean 300 || echo "$SELF: couldn't remove the Mac's scratch $RELATIVE" >&2
  fi
  exit "$RESULT_EXIT"
}

RESULT_EXIT=1 RESULT_QUEUED=- RESULT_STARTED=- RESULT_ENDED=- TREE=""
case "$OP" in
  list) remote_call list 120 ;;
  clean) remote_call clean 300 ;;
  cancel) remote_call cancel 120 ;;
  wait)
    began="$(date +%s)"
    STARTED=1
    wait_run
    STARTED=0
    collect "$began" -
    finish
    ;;
  run)
    began="$(date +%s)"
    CLAIMED=1
    TREE="$(copy_tree)"
    copied="$(($(date +%s) - began)) s"
    printf -v command_words '%q ' "${COMMAND[@]}"
    # the archive's build number: the scratch copy has no Git history (a --env of the same name comes after it)
    env_lines=""
    build_number="$(git -C "$NATIVE_ROOT" rev-list --count HEAD 2>/dev/null || true)"
    [[ ! "$build_number" =~ ^[0-9]+$ ]] || env_lines="export BUILD_IOS_BUILD_NUMBER=$build_number"$'\n'
    for pair in ${ENVS[@]+"${ENVS[@]}"}; do
      env_lines+="export ${pair%%=*}=$(printf %q "${pair#*=}")"$'\n'
    done
    STARTED=1
    pid="$(remote_call start 120 "STAMP=$TREE" "COMMAND=$command_words" "ENVS=$env_lines" "TIMEOUT=$TIMEOUT" \
      "QUEUE_TIMEOUT=$QUEUE_TIMEOUT")" || fail "couldn't start the run on the Mac"
    echo "$SELF: $NAME: tree $TREE copied in $copied; run started on the Mac (pid $pid)" >&2
    wait_run
    STARTED=0
    collect "$began" "$copied"
    finish
    ;;
esac
