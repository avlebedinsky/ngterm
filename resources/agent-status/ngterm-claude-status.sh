#!/usr/bin/env bash
# Claude Code status adapter installed by ngterm's Help ▸ Install Agent Status Hooks command.
#
# A worker agent spawned from inside a session (headless `claude -p` from a tool call, a second CLI
# agent) inherits the spawner's NGTERM_* environment, so its own hooks would repaint the SPAWNER's
# row. This adapter keeps that ownership question inside the installed hook package: decide from
# process topology, not terminal state. The hook is a descendant of the agent that fired it, so
# exactly one agent between here and the pane means "I am the pane's agent"; a second one means
# another agent spawned mine, so stay silent. A consecutive run of one name counts once, because a
# launcher that runs the real binary without `exec` leaves one agent wearing several pids. A worker
# is therefore caught only when a surviving process, such as its tool call's shell, separates it
# from a same-name spawner. A direct child is accepted as reporting: a hook whose whole command is
# `claude -p ...` runs through `sh -c`, which execs it, and that worker repaints the owner's row.
# A tty test cannot answer this — a headless lane that legitimately owns its pane has none, and a
# worker under script/expect gets a fresh pty anyway.
#
# It fails OPEN (reports) whenever the chain is unreadable or severed, e.g. a detached worker whose
# spawner already exited: a missed guard is the behavior without this adapter, while a false silence
# is a bug with no symptom. The argv reaches the shared wrapper unchanged except for the two payload
# rewrites at the end.
set -u

# A background job of Claude Code's daemon (CLAUDE_JOB_DIR is set: Claude Code moves a conversation there on
# /background or on a Left arrow at its empty prompt) has the daemon's NGTERM_*, which name the pane the daemon was
# started in: a session long gone, or another one's. `ngtermctl agent rerun` finds the pane that holds this
# conversation from the payload and runs this hook again as that pane; with none it runs nothing, silently.
if [ -n "${CLAUDE_JOB_DIR:-}" ] && [ -z "${NGTERM_AGENT_LOCATED:-}" ]; then
  command -v "${NGTERMCTL:-ngtermctl}" >/dev/null 2>&1 || exit 0
  rerun=(agent rerun claude)
  [ -n "${NGTERM_SOCKET:-}" ] && rerun+=(--socket "$NGTERM_SOCKET")
  exec "${NGTERMCTL:-ngtermctl}" "${rerun[@]}" -- "${BASH:-bash}" "$0" "$@"
fi

[ -n "${NGTERM_SESSION_ID:-}" ] || exit 0

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd)
status_wrapper=${NGTERM_STATUS_WRAPPER:-"$script_dir/ngterm-agent-status.sh"}

# exact names, not a glob: `claude*` also matches wrapper scripts such as claude-opus-worker.
# The list is the union of the hook-driven agents and shell/integration.sh's NGTERM_AGENT_RE
# default — one agent set split by detection mechanism, and the walk must count all of it: an
# integration-driven agent (say gemini) that spawns `claude -p` is a spawner too, and missing it
# here would let the worker's reset repaint the gemini pane. Keep the two lists in sync.
#
# An overridden NGTERM_AGENT_RE (integration.sh documents the export) extends the set beyond the
# literals, matched against the same argv[0] basename the literal list sees — not the typed command
# line the shell integration matched, so an override that inspects arguments won't extend the walk.
# The expression is evaluated as ERE; zsh under RE_MATCH_PCRE and fish read the same value as PCRE,
# so an override can be valid there and invalid here. `[[` then returns 2, which lands on "not an
# agent" and keeps the walk fail-open; the redirect swallows bash's complaint about the expression.
is_agent() {
  case "$1" in claude | codex | kimi | opencode | pi | omp | gemini | cursor-agent | aider | crush | goose) return 0 ;; esac
  if [ -n "${NGTERM_AGENT_RE:-}" ]; then
    [[ $1 =~ $NGTERM_AGENT_RE ]] 2>/dev/null && return 0
  fi
  return 1
}

agents=0
prev_agent=
p=$PPID
for _ in 1 2 3 4 5 6 7 8; do
  [ -n "$p" ] && [ "$p" -gt 1 ] 2>/dev/null || break
  line=$(ps -o ppid=,command= -p "$p" 2>/dev/null) || break
  [ -n "$line" ] || break
  line=${line#"${line%%[![:space:]]*}"} # ltrim the ppid column's padding
  ppid=${line%% *}
  cmd=${line#* }

  base=${cmd%% *}
  base=${base##*/} # argv[0] basename
  base=${base#-}   # login shells report -/bin/zsh
  base=${base#\(}
  base=${base%\)}  # macOS renders (name) when argv is unreadable
  case "$base" in # node/bun-hosted CLIs present as `node .../cli.js`, so test argv[1] too
    node | bun | deno | python3 | python)
      rest=${cmd#* }
      hosted=${rest%% *}
      hosted=${hosted##*/}
      is_agent "$hosted" && base=$hosted
      ;;
  esac

  if is_agent "$base"; then
    [ "$base" = "$prev_agent" ] || agents=$((agents + 1))
    prev_agent=$base
  else
    prev_agent=
  fi
  [ "$agents" -gt 1 ] && exit 0        # a second agent above mine: I am a worker, not the pane's agent
  # the pane boundary: the app, a headless origin, or the session server that holds a live session's shell. Whatever
  # is above started the origin (an agent's shell that ran ngterm-headless, say) and is not in the pane.
  case "$base" in login | ngterm | ngterm-headless | ngterm-session) break ;; esac
  p=$ppid
done

# Pipes $payload to `ngtermctl agent hook claude <event>`, the pane addressed as the status wrapper addresses it.
report_agent() {
  local args=(agent hook claude "$1" --target "$NGTERM_SESSION_ID")
  [ -n "${NGTERM_PANE:-}" ] && args+=(--pane "$NGTERM_PANE")
  [ -n "${NGTERM_PANE_ID:-}" ] && args+=(--pane-id "$NGTERM_PANE_ID")
  [ -n "${NGTERM_SOCKET:-}" ] && args+=(--socket "$NGTERM_SOCKET")
  printf '%s' "$payload" | "${NGTERMCTL:-ngtermctl}" "${args[@]}" >/dev/null 2>&1 || true
}

export NGTERM_STATUS_WALKED=1

# the completed->active rewrite drops the caller's flags: a carried --auto-reset would idle the row on the
# next visit while a background agent still runs. stdin is read only when it is not a terminal. Its first
# 4096 characters wait 2 s at most, so an inherited open pipe that sends nothing never hangs a hook; a full
# head or a chat record reads the rest to EOF, as its writer closes the pipe. bash 3.2's read returns 1 at
# EOF and on timeout alike, so the payload is tested, not the status.
payload=
if [ ! -t 0 ]; then
  IFS= read -r -d '' -n 4096 -t 2 payload || true
  record_re='"hook_event_name"[[:space:]]*:[[:space:]]*"(UserPromptSubmit|PermissionRequest)"'
  if [ "${CLAUDE_CODE_ENTRYPOINT:-}" = cli ] &&
    { [ "${1:-}" = active ] || [ "${1:-}" = blocked ]; } && [[ $payload =~ $record_re ]]; then
    event=${BASH_REMATCH[1]}
    rest=$(cat)
    payload=$payload$rest
    subagent_re='(^|[^\\])"agent_id"[[:space:]]*:[[:space:]]*"[^"]'
    if [ "$event" = PermissionRequest ] || ! [[ $payload =~ $subagent_re ]]; then
      report_agent "$event"
    fi
  elif [ "${#payload}" -eq 4096 ]; then
    # a full head is a payload whose writer closes the pipe, so cat ends, where read takes 2 s for 150 KB
    rest=$(cat)
    payload=$payload$rest
  fi
fi

field() { # key path value, array length for an array, empty when absent or unparseable
  if [[ $OSTYPE == darwin* ]] && [ -x /usr/bin/plutil ]; then
    if v=$(printf '%s' "$payload" | /usr/bin/plutil -extract "$1" raw -o - - 2>/dev/null); then printf '%s' "$v"; fi
  elif command -v python3 >/dev/null 2>&1; then
    printf '%s' "$payload" | python3 -c 'import json, sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if isinstance(value, list) else value[key]
if isinstance(value, list): print(len(value), end="")
elif isinstance(value, (str, int, float)) and not isinstance(value, bool): print(value, end="")' "$1" 2>/dev/null
  elif command -v jq >/dev/null 2>&1; then
    printf '%s' "$payload" | jq -r --arg p "$1" \
      'getpath($p | split(".") | map(tonumber? // .)) | if type == "array" then length else . // empty end' 2>/dev/null
  fi
}

running_agent() {
  n=$(field background_tasks)
  case "$n" in '' | *[!0-9]*) return 1 ;; esac
  i=0
  while [ "$i" -lt "$n" ]; do
    if [ "$(field "background_tasks.$i.status")" = running ]; then
      case "$(field "background_tasks.$i.type")" in subagent | workflow | shell) return 0 ;; esac
    fi
    i=$((i + 1))
  done
  return 1
}

if [ -n "$payload" ]; then
  if [ -n "$(field agent_id)" ]; then
    [ "${1:-}" = active ] && set -- "$@" --if-idle
  elif [ "${1:-}" = completed ] && [ "$(field hook_event_name)" = Stop ] && running_agent; then
    set -- active --blink
  fi
fi

exec "$status_wrapper" "$@"
