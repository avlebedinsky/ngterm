#!/usr/bin/env bash
# The Android app's end-to-end tests (mobile/android/app/src/androidTest/.../e2e) on a device or emulator, against a
# real machine: a private ngterm-headless behind a private, unprivileged sshd on 127.0.0.1, which the device reaches
# through `adb reverse`. It builds ngterm-headless, ngtermctl and ngterm-session, stages them (scripts/stage.sh) into a
# scratch directory under cargo's target dir, starts the instance in a private state dir and home there (every
# NGTERM_* stripped, so nothing it starts can reach an ngterm the script was started from), makes a throwaway host key
# and client key, and runs the instrumented tests through scripts/build-android.sh with the machine's address, user and
# client key as instrumentation arguments (the tests import the key and add the machine as a person does).
#
# The machine's PATH also holds `e2e-claude`: the mobile e2e's stand-in Claude Code in its standalone form
# (crates/ngterm-mobile/tests/support/fake_agent.py --standalone, on the system's python3), which a test starts in a
# session's shell as a person starts Claude Code (ChatFirstMessageE2eTest).
#
# The same sshd answers on a second port as a Mac running upstream agterm (AgtermPeerE2eTest): its logins get a home
# and a PATH of their own holding only the conformance suite's stand-in agterm
# (conformance/agterm_conformance/fake_agterm.py: its `agtermctl` and bundled `zmx`, no `ngtermctl`), seeded with one
# session; no live agterm is involved.
#
# On a third port it answers as one host that runs both apps (BothAppsE2eTest): its logins see this run's ngterm and a
# second stand-in agterm with a session of its own, so a listing asks both.
#
# On a fourth port it answers as a Mac whose agterm is not in Live sessions mode (AgtermNotLiveE2eTest): a third
# stand-in agterm with restore mode none and no session, so the app says why it lists nothing.
#
# On exit it stops the instance and sshd by the pids they recorded, ends the stand-in agterms' processes (those with
# their private homes), ends every session of the private namespace through that namespace's own session server and
# removes the namespace's directory, and drops the port reverses. The scratch directory is removed after a pass and
# kept after a failure (headless.log, sshd/sshd.log). With --evidence DIR, the tests' screenshots and the
# device's log of the run are copied into DIR.
#
# On the phone AVD scripts/setup-android.sh made, the script first puts the emulator in a phone's shape (the display's
# own density, no hardware keyboard), so the tests meet the soft keyboard and the extra-keys row a phone user has; it
# holds until the emulator restarts and needs no undoing. Any other device or AVD runs as it is.
#
# Usage: scripts/test-android-e2e.sh [--profile release|debug] [--evidence DIR] [TEST...]
#   TEST: a class or Class#method of com.nickalie.ngterm.android.e2e (default: every class of the package), each
#   run on its own
#   One device: the one adb sees, or ANDROID_SERIAL's. MOBILE_E2E_WORK_ROOT overrides where the scratch directory is
#   made (default: cargo's target dir); keep it short, since the private control socket lives under it.
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

E2E_PACKAGE=com.nickalie.ngterm.android.e2e
# where the tests save their screenshots on the device (SoftKeyboardSwitchE2eTest's EVIDENCE_DIR)
DEVICE_EVIDENCE=/data/local/tmp/ngterm-e2e

usage() {
  die "usage: scripts/test-android-e2e.sh [--profile release|debug] [--evidence DIR] [TEST...]"
}

PROFILE=release
EVIDENCE=
TESTS=()
while (($#)); do
  case "$1" in
    --profile)
      (($# >= 2)) && [[ "$2" == debug || "$2" == release ]] || usage
      PROFILE="$2"
      shift 2
      ;;
    --evidence)
      (($# >= 2)) || usage
      EVIDENCE="$2"
      shift 2
      ;;
    -h | --help) usage ;;
    -*) usage ;;
    *)
      TESTS+=("$1")
      shift
      ;;
  esac
done
[[ "$(host_os)" == linux ]] || die "the Android e2e test runs on Linux"
# the evidence directory is made and resolved now, before anything starts and before the cd below: a path it cannot
# make or write stops the run here, not in the exit trap
if [[ -n "$EVIDENCE" ]]; then
  mkdir -p "$EVIDENCE" 2>/dev/null && [[ -w "$EVIDENCE" ]] || die "cannot write the evidence directory $EVIDENCE"
  EVIDENCE="$(cd "$EVIDENCE" && pwd)"
fi

cd "$NATIVE_ROOT"
eval "$(scripts/setup-android.sh --check)"
ADB="$ANDROID_HOME/platform-tools/adb"
[[ -x "$ADB" ]] || ADB="$(command -v adb)" || die "no adb"
devices="$("$ADB" devices | awk 'NR > 1 && $2 == "device" { print $1 }')"
if [[ -z "${ANDROID_SERIAL:-}" ]]; then
  [[ -n "$devices" ]] || die "no device or emulator: start one (an AVD scripts/setup-android.sh made)"
  (($(wc -l <<<"$devices") == 1)) || die "adb sees several devices: pick one with ANDROID_SERIAL"
  export ANDROID_SERIAL="$devices"
fi

# The phone AVD as a phone in the hand: the display's own density, and no hardware keyboard, so the soft keyboard and
# the extra-keys row show. The system image attaches a PC keyboard whatever the AVD's config says, so its driver lets
# go of it (as root, which the emulator's image gives) until the emulator restarts. Any other device stays as it is.
phone_shape() {
  local avd config rooted i
  [[ "$ANDROID_SERIAL" == emulator-* ]] || return 0
  avd="$("$ADB" emu avd name 2>/dev/null | head -n 1 | tr -d '\r')"
  [[ "$avd" == "$ANDROID_AVD_PHONE" ]] || return 0
  if [[ "$("$ADB" shell wm density)" == *Override* ]]; then
    "$ADB" shell wm density reset >/dev/null
    settle
  fi
  config="$("$ADB" shell am get-config)"
  if [[ "${config%%$'\n'*}" != *-nokeys-* ]]; then
    rooted="$("$ADB" shell id -u | tr -d '\r')"
    as_root 0 || die "no root on $avd to detach its hardware keyboard"
    # shellcheck disable=SC2016 # the device's shell expands it
    "$ADB" shell 'for k in /sys/bus/serio/drivers/atkbd/serio*; do
      [ -e "$k" ] && echo -n "${k##*/}" >/sys/bus/serio/drivers/atkbd/unbind; done' || true
    [[ "$rooted" == 0 ]] || as_root 2000 || die "adb did not come back from root on $avd"
    for ((i = 0; i < 50; i++)); do
      config="$("$ADB" shell am get-config)"
      [[ "${config%%$'\n'*}" == *-nokeys-* ]] && break
      ((i < 49)) || die "$avd still has a hardware keyboard (${config%%$'\n'*})"
      sleep 0.2
    done
  fi
  echo "→ $avd: phone shape ($("$ADB" shell wm density | tr '\n' ' '), no hardware keyboard)" >&2
}
# A density change restarts the launcher and the system's UI, which may be slow enough for the system to offer to
# close one over the app: wait until no such dialog has stood for five seconds, closing those that come.
settle() {
  local i windows quiet=0
  for ((i = 0; i < 60 && quiet < 5; i++)); do
    sleep 1
    windows="$("$ADB" shell dumpsys window windows)"
    if [[ "$windows" == *'Application Not Responding'* ]]; then
      quiet=0
      "$ADB" shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS >/dev/null 2>&1 || true
    else
      quiet=$((quiet + 1))
    fi
  done
  ((quiet == 5)) || die "$ANDROID_AVD_PHONE keeps a not-responding dialog up after its density change"
}
# adbd as root ($1 0) or as the shell user again ($1 2000); it restarts, so the device answers a moment later
as_root() {
  local i
  if [[ "$1" == 0 ]]; then "$ADB" root >/dev/null 2>&1 || true; else "$ADB" unroot >/dev/null 2>&1 || true; fi
  for ((i = 0; i < 50; i++)); do
    [[ "$(timeout 5 "$ADB" shell id -u 2>/dev/null | tr -d '\r')" == "$1" ]] && return 0
    sleep 0.2
  done
  return 1
}
phone_shape

SSHD=
for candidate in "$(command -v sshd 2>/dev/null || true)" /usr/sbin/sshd /usr/local/sbin/sshd /usr/bin/sshd; do
  [[ -n "$candidate" && -x "$candidate" ]] && SSHD="$candidate" && break
done
[[ -n "$SSHD" ]] || die "no sshd on this machine"

TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
cargo build -p ngterm-headless -p ngterm-ctl -p ngterm-session

ROOT="${MOBILE_E2E_WORK_ROOT:-$TARGET}"
mkdir -p "$ROOT"
WORK="$(mktemp -d "$ROOT/ae2e.XXXXXX")"
WORK="$(cd "$WORK" && pwd)"
USER_NAME="$(id -un)"
PRIVATE=(env -i "NGTERM_STATE_DIR=$WORK/s" "HOME=$WORK/home" "PATH=$WORK/bin:$WORK/stage/bin:/usr/bin:/bin"
  SHELL=/bin/bash "USER=$USER_NAME" "LOGNAME=$USER_NAME" LANG=C.UTF-8)
PORT=
AGTERM_PORT=
BOTH_PORT=
OFF_PORT=
AGTERM="$WORK/agterm"
BOTH="$WORK/both"
OFF="$WORK/agterm-off"

# of the given pids (all this user's when none is given), those of the processes the logins of the stand-in agterm
# machines (the agterm Mac, the not-Live Mac) and of the host that runs both started: each has that machine's private
# home
agterm_procs() {
  local pid pids=("$@")
  (($#)) || mapfile -t pids < <(pgrep -u "$(id -u)" 2>/dev/null || true)
  for pid in "${pids[@]}"; do
    grep -qzxF -e "HOME=$AGTERM/home" -e "HOME=$BOTH/home" -e "HOME=$OFF/home" "/proc/$pid/environ" 2>/dev/null &&
      echo "$pid"
  done
  return 0
}

# The instance and sshd, by the pids they recorded and only while those pids still run this run's binary or config; the
# private namespace's live sessions, ended through its own session server, and its directory once that server, left
# empty, has gone (after its idle grace; the namespace is this run's alone, hashed from its private state dir); the
# device's port reverse.
reap() {
  local session="$WORK/stage/bin/ngterm-session" pid names namespace i pids
  if [[ -s "$WORK/headless.pid" ]]; then
    pid="$(<"$WORK/headless.pid")"
    if [[ "$(readlink -f "/proc/$pid/exe" 2>/dev/null)" == "$(readlink -f "$WORK/stage/bin/ngterm-headless")" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
  if [[ -s "$WORK/sshd/sshd.pid" ]]; then
    pid="$(<"$WORK/sshd/sshd.pid")"
    if tr '\0' ' ' 2>/dev/null <"/proc/$pid/cmdline" | grep -qF "$WORK/sshd/sshd_config"; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
  if [[ -s "$WORK/live-dir" && -x "$session" ]]; then
    namespace="$(<"$WORK/live-dir")"
    names="$("${PRIVATE[@]}" "NGTERM_LIVE_DIR=$namespace" "$session" list --short 2>/dev/null || true)"
    # shellcheck disable=SC2086 # one session name per word
    [[ -z "$names" ]] || "${PRIVATE[@]}" "NGTERM_LIVE_DIR=$namespace" "$session" kill --force $names \
      >/dev/null 2>&1 || true
    for ((i = 0; i < 300; i++)); do
      [[ -e "$namespace/server.sock" ]] || break
      sleep 0.1
    done
    if [[ -e "$namespace/server.sock" ]]; then
      echo "→ the session server of $namespace is still running; it and the directory are left behind" >&2
    else
      rm -rf "$namespace"
    fi
  fi
  # the stand-in agterms' streams and shells, which outlive a connection the device dropped unclosed: every process
  # their logins started has their machine's private home. An interactive shell ignores TERM, so HUP goes with it and
  # KILL ends what is left a moment later
  pids="$(agterm_procs)"
  # shellcheck disable=SC2086 # one pid per word
  if [[ -n "$pids" ]]; then
    kill -HUP $pids 2>/dev/null || true
    kill -TERM $pids 2>/dev/null || true
    for ((i = 0; i < 20; i++)); do
      pids="$(agterm_procs $pids)"
      [[ -n "$pids" ]] || break
      sleep 0.1
    done
    [[ -z "$pids" ]] || kill -KILL $pids 2>/dev/null || true
  fi
  [[ -z "$PORT" ]] || "$ADB" reverse --remove "tcp:$PORT" >/dev/null 2>&1 || true
  [[ -z "$AGTERM_PORT" ]] || "$ADB" reverse --remove "tcp:$AGTERM_PORT" >/dev/null 2>&1 || true
  [[ -z "$BOTH_PORT" ]] || "$ADB" reverse --remove "tcp:$BOTH_PORT" >/dev/null 2>&1 || true
  [[ -z "$OFF_PORT" ]] || "$ADB" reverse --remove "tcp:$OFF_PORT" >/dev/null 2>&1 || true
}
collect() {
  [[ -n "$EVIDENCE" ]] || return 0
  mkdir -p "$EVIDENCE" 2>/dev/null || {
    echo "→ cannot make $EVIDENCE; no evidence collected" >&2
    return 0
  }
  "$ADB" pull "$DEVICE_EVIDENCE/." "$EVIDENCE/" >/dev/null 2>&1 || true
  "$ADB" logcat -d >"$EVIDENCE/logcat.txt" 2>/dev/null || true
  cp "$WORK/headless.log" "$EVIDENCE/" 2>/dev/null || true
}
on_exit() {
  local status=$?
  # reap first: whatever collect meets, the processes, the namespace and the port reverse never stay behind
  reap
  collect
  if ((status == 0)); then
    rm -rf "$WORK"
  else
    echo "→ android e2e failed; its scratch directory is kept: $WORK" >&2
  fi
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

SOCKET="$WORK/s/ngterm.sock"
((${#SOCKET} <= 103)) || die "the control socket path is too long ($SOCKET): set a shorter MOBILE_E2E_WORK_ROOT"
scripts/stage.sh "$WORK/stage" --profile debug
mkdir -p "$WORK/s" "$WORK/home" "$WORK/sshd" "$WORK/bin"
printf '#!/bin/sh\nexec %q -I %q --standalone "$@"\n' "$(command -v python3)" \
  "$NATIVE_ROOT/crates/ngterm-mobile/tests/support/fake_agent.py" >"$WORK/bin/e2e-claude"
chmod 755 "$WORK/bin/e2e-claude"

# the instance's live namespace: /tmp/ngterm-live-<FNV-1a 64 of the resolved state dir> (ngterm_core::live)
python3 -I - "$WORK/s" >"$WORK/live-dir" <<'EOF'
import os, sys
h = 14695981039346656037
for b in os.fsencode(os.path.realpath(sys.argv[1])):
    h = ((h ^ b) * 1099511628211) % (1 << 64)
print(f"/tmp/ngterm-live-{h:016x}", end="")
EOF

"${PRIVATE[@]}" "$WORK/stage/bin/ngterm-headless" --skip-agent-setup </dev/null >"$WORK/headless.log" 2>&1 &
echo $! >"$WORK/headless.pid"
for ((i = 0; i < 300; i++)); do
  "${PRIVATE[@]}" "$WORK/stage/bin/ngtermctl" live tree --json --socket "$SOCKET" >/dev/null 2>&1 && break
  ((i < 299)) || die "the private ngterm-headless never answered: $WORK/headless.log"
  sleep 0.1
done

ssh-keygen -q -t ed25519 -N '' -C ngterm-e2e-host -f "$WORK/sshd/host_ed25519"
ssh-keygen -q -t ed25519 -N '' -C ngterm-e2e-client -f "$WORK/client_ed25519"
cp "$WORK/client_ed25519.pub" "$WORK/sshd/authorized_keys"
cat >"$WORK/force-command.sh" <<EOF
#!/bin/sh
export NGTERM_STATE_DIR='$WORK/s'
export HOME='$WORK/home'
export PATH='$WORK/bin:$WORK/stage/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/force-command.sh"

# a stand-in agterm in the directory $1: agterm's CLI in its bin and its app bundle's zmx, as conformance's AgtermMac
# lays them out, with one live session named $2 (none when it is empty), in restore mode $3 (default live)
FAKE_AGTERM="$NATIVE_ROOT/conformance/agterm_conformance/fake_agterm.py"
PYTHON="$(command -v python3)"
stand_in_agterm() {
  local root="$1" program wrapper
  mkdir -p "$root/bin" "$root/agterm.app" "$root/home" "$root/zmx-sockets"
  for program in agtermctl zmx; do
    wrapper="$root/bin/agtermctl"
    [[ "$program" == zmx ]] && wrapper="$root/agterm.app/zmx"
    printf '#!/bin/sh\nexec %q -I %q %q %s "$@"\n' "$PYTHON" "$FAKE_AGTERM" "$root" "$program" >"$wrapper"
    chmod 755 "$wrapper"
  done
  python3 -I - "$FAKE_AGTERM" "$root" "$2" "${3:-live}" <<'PY'
import importlib.util, os, sys, uuid
spec = importlib.util.spec_from_file_location("fake_agterm", sys.argv[1])
fake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fake)
root = sys.argv[2]
daemon = f"agterm-{uuid.uuid4().hex}"
window = {"id": str(uuid.uuid4()).upper(), "name": "Main"}
workspace = {"id": str(uuid.uuid4()).upper(), "name": "work"}
session = {
    "id": str(uuid.uuid4()).upper(), "name": sys.argv[3],
    "windowID": window["id"], "windowName": window["name"],
    "workspaceID": workspace["id"], "workspaceName": workspace["name"],
    "cwd": os.path.join(root, "home"), "panes": [{"pane": "left", "daemon": daemon}],
}
listed = bool(sys.argv[3])
fake.save_state(fake.Path(root), {
    "running": True, "restore": sys.argv[4], "home": os.path.join(root, "home"),
    "zmx": os.path.join(root, "agterm.app", "zmx"), "socketDirectory": os.path.join(root, "zmx-sockets"),
    "window": window, "workspace": workspace,
    "sessions": [session] if listed else [], "daemons": [daemon] if listed else [],
})
PY
}

# the stand-in agterm Mac: its logins never see this build's binaries
stand_in_agterm "$AGTERM" qa-agterm-work
cat >"$WORK/agterm-command.sh" <<EOF
#!/bin/sh
export HOME='$AGTERM/home'
export PATH='$AGTERM/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/agterm-command.sh"

# the host that runs both apps: this run's ngterm, and beside it a stand-in agterm of its own, in one login
stand_in_agterm "$BOTH" qa-both-agterm
cat >"$WORK/both-command.sh" <<EOF
#!/bin/sh
export NGTERM_STATE_DIR='$WORK/s'
export HOME='$BOTH/home'
export PATH='$WORK/stage/bin:$BOTH/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/both-command.sh"

# the Mac whose agterm is not in Live sessions mode: no session, restore mode none
stand_in_agterm "$OFF" "" none
cat >"$WORK/agterm-off-command.sh" <<EOF
#!/bin/sh
export HOME='$OFF/home'
export PATH='$OFF/bin:/usr/bin:/bin'
exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
EOF
chmod 755 "$WORK/agterm-off-command.sh"

# a free port is taken back from the kernel before sshd binds it, so another process may win it: three tries
for ((try = 0; try < 3; try++)); do
  read -r PORT AGTERM_PORT BOTH_PORT OFF_PORT < <(python3 -I -c '
import socket
sockets = [socket.socket() for _ in range(4)]
for one in sockets:
    one.bind(("127.0.0.1", 0))
print(*(one.getsockname()[1] for one in sockets))')
  cat >"$WORK/sshd/sshd_config" <<EOF
ListenAddress 127.0.0.1
Port $PORT
Port $AGTERM_PORT
Port $BOTH_PORT
Port $OFF_PORT
HostKey "$WORK/sshd/host_ed25519"
PidFile "$WORK/sshd/sshd.pid"
AuthorizedKeysFile "${WORK//%/%%}/sshd/authorized_keys"
AuthenticationMethods publickey
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
AllowUsers $USER_NAME
StrictModes no
PermitUserRC no
PermitUserEnvironment no
PermitTTY yes
AllowTcpForwarding no
AllowAgentForwarding no
X11Forwarding no
PrintMotd no
ForceCommand '$WORK/force-command.sh'
LogLevel VERBOSE
Match LocalPort $AGTERM_PORT
  ForceCommand '$WORK/agterm-command.sh'
Match LocalPort $BOTH_PORT
  ForceCommand '$WORK/both-command.sh'
Match LocalPort $OFF_PORT
  ForceCommand '$WORK/agterm-off-command.sh'
EOF
  env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin "$SSHD" -D -f "$WORK/sshd/sshd_config" -E "$WORK/sshd/sshd.log" \
    </dev/null >/dev/null 2>&1 &
  for ((i = 0; i < 150; i++)); do
    banner="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$PORT && head -c 4 <&3" 2>/dev/null || true)"
    other="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$AGTERM_PORT && head -c 4 <&3" 2>/dev/null || true)"
    third="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$BOTH_PORT && head -c 4 <&3" 2>/dev/null || true)"
    fourth="$(timeout 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$OFF_PORT && head -c 4 <&3" 2>/dev/null || true)"
    [[ "$banner" == SSH- && "$other" == SSH- && "$third" == SSH- && "$fourth" == SSH- ]] && break 2
    sleep 0.1
  done
  [[ -s "$WORK/sshd/sshd.pid" ]] && kill -TERM "$(<"$WORK/sshd/sshd.pid")" 2>/dev/null || true
  PORT=
  AGTERM_PORT=
  BOTH_PORT=
  OFF_PORT=
done
[[ -n "$PORT" ]] || die "sshd never answered: $WORK/sshd/sshd.log"
"$ADB" reverse "tcp:$PORT" "tcp:$PORT" >/dev/null
"$ADB" reverse "tcp:$AGTERM_PORT" "tcp:$AGTERM_PORT" >/dev/null
"$ADB" reverse "tcp:$BOTH_PORT" "tcp:$BOTH_PORT" >/dev/null
"$ADB" reverse "tcp:$OFF_PORT" "tcp:$OFF_PORT" >/dev/null

"$ADB" shell rm -rf "$DEVICE_EVIDENCE"
"$ADB" shell mkdir -p "$DEVICE_EVIDENCE"
"$ADB" logcat -c || true

# each test (by default each class of the package) in an instrumentation run of its own: a run installs the app anew,
# so every test starts from a fresh process, as each signs in from a fresh install and the app's stores keep what they
# read in memory for the process's life
if ((${#TESTS[@]} == 0)); then
  for source in mobile/android/app/src/androidTest/kotlin/com/nickalie/ngterm/android/e2e/*E2eTest.kt; do
    TESTS+=("$(basename "$source" .kt)")
  done
fi
for test in "${TESTS[@]}"; do
  echo "→ $test" >&2
  scripts/build-android.sh --profile "$PROFILE" :app:connectedGithubDebugAndroidTest \
    -Pandroid.testInstrumentationRunnerArguments.class="$E2E_PACKAGE.$test" \
    -Pandroid.testInstrumentationRunnerArguments.e2eHost=127.0.0.1 \
    -Pandroid.testInstrumentationRunnerArguments.e2ePort="$PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eAgtermPort="$AGTERM_PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eBothPort="$BOTH_PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eAgtermOffPort="$OFF_PORT" \
    -Pandroid.testInstrumentationRunnerArguments.e2eUser="$USER_NAME" \
    -Pandroid.testInstrumentationRunnerArguments.e2eKey="$(base64 -w0 "$WORK/client_ed25519")"
done
echo "→ android e2e passed" >&2
