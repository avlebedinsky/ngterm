#!/usr/bin/env bash
# ngterm-agent-status — set the current ngterm session's agent-status indicator.
#
#   ngterm-agent-status.sh active            # agent is busy
#   ngterm-agent-status.sh completed         # agent finished a turn
#   ngterm-agent-status.sh blocked  --blink  # agent is waiting on you (pulse for attention)
#   ngterm-agent-status.sh idle              # clear the indicator
#
# States: idle | active | completed | blocked. An optional --blink / --auto-reset
# (and any further args) is forwarded verbatim to `ngtermctl session status`.
#
# Outside ngterm this is a silent no-op, so it is safe to call from any hook.
#
# As a hook it must never interfere with the agent: stdout/stderr are suppressed
# (Claude Code injects a UserPromptSubmit/SessionStart hook's stdout into the
# prompt context) and it always exits 0 (a non-zero exit can block the turn).
#
# ngtermctl resolution order (the binary that talks to the control socket):
#   1. $NGTERMCTL — an explicit override the caller set.
#   2. the absolute bundled-binary path the installer bakes in: the installer
#      rewrites the NGTERMCTL default below to the app's bundled bin/ngtermctl,
#      so the hook fires even when the CLI was never symlinked into PATH.
#   3. `ngtermctl` on PATH — the fallback when no override was set and the baked
#      path no longer exists, which is what a bundle moved since the install leaves
#      behind (installing from the mounted DMG bakes a /Volumes path, dead on eject).
set -u

[ -n "${NGTERM_SESSION_ID:-}" ] || exit 0   # not inside ngterm: nothing to do

# ngterm's spawner guard (an ngterm extension: upstream's wrapper has none). An agent that another agent started
# inside a pane inherits that pane's NGTERM_*, so its reports would paint the spawner's row. The Claude and Codex
# adapters decide that themselves and say so (NGTERM_STATUS_WALKED); everything else that reports through this wrapper
# alone (the Pi and Oh My Pi extensions, the OpenCode plugins: a `pi -p`, an `omp` or an `opencode run` under another
# agent) is decided here, by the Claude adapter's ancestor walk (ngterm-claude-status.sh explains it), with the same
# agent names and the same pane boundary: keep them in step. A second agent above the caller and below the pane's
# origin makes the call a worker's, which is dropped; an unreadable chain reports (fail open). The PowerShell form has
# no such walk yet: the cost of a process-table read there, on each report, is not measured.
is_agent() {
  case "$1" in claude | codex | kimi | opencode | pi | omp | gemini | cursor-agent | aider | crush | goose) return 0 ;; esac
  if [ -n "${NGTERM_AGENT_RE:-}" ]; then
    [[ $1 =~ $NGTERM_AGENT_RE ]] 2>/dev/null && return 0
  fi
  return 1
}

if [ -z "${NGTERM_STATUS_WALKED:-}" ]; then
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
    [ "$agents" -gt 1 ] && exit 0 # a second agent above the caller's: a worker's report, not the pane's agent's
    # the pane boundary: the app, a headless origin, or the session server that holds a live session's shell
    case "$base" in login | ngterm | ngterm-headless | ngterm-session) break ;; esac
    p=$ppid
  done
fi

# --socket is a SUBCOMMAND option, so it must come AFTER `session status`, not before
# it. Pass it only when NGTERM_SOCKET is set (the app injects it alongside the id).
state=$1
shift

# forward the pane discriminators when the app injected them: each session surface
# (main/split/scratch) sets its own NGTERM_PANE (the role) plus NGTERM_PANE_ID (a stable
# per-surface token). the role can go stale — a split survivor promoted into the main pane
# keeps its baked `right` — so we also forward the token as --pane-id, which the app resolves
# to the surface's CURRENT slot and lets override the stale role (#199). both are validated
# ngtermctl-side, so pass them through verbatim. the ${arr[@]+..} guard keeps the empty-array
# expansion safe under `set -u` on bash 3.2.
pane_args=()
[ -n "${NGTERM_PANE:-}" ] && pane_args+=(--pane "$NGTERM_PANE")
[ -n "${NGTERM_PANE_ID:-}" ] && pane_args+=(--pane-id "$NGTERM_PANE_ID")

if [ -n "${NGTERM_SOCKET:-}" ]; then
  "${NGTERMCTL:-ngtermctl}" session status "$state" \
    --target "$NGTERM_SESSION_ID" --socket "$NGTERM_SOCKET" \
    "${pane_args[@]+"${pane_args[@]}"}" "$@" >/dev/null 2>&1 || true
else
  "${NGTERMCTL:-ngtermctl}" session status "$state" \
    --target "$NGTERM_SESSION_ID" "${pane_args[@]+"${pane_args[@]}"}" "$@" >/dev/null 2>&1 || true
fi
exit 0
