#!/usr/bin/env bash
# Codex lifecycle hook installed by ngterm's Help ▸ Install Agent Status Hooks command.
#
# Codex fires PermissionRequest before Auto Review decides whether a person must approve. Treating
# that raw event as blocked therefore false-flags automatically reviewed tools. This hook keeps the
# agent-specific workaround inside the installed hook package: one watcher per ngterm pane reads the
# live visible footer for dialogs, while Stop checks the final assistant message for a question.
#
# ngterm's chat view (an ngterm extension): SessionStart, UserPromptSubmit and PermissionRequest also report to the
# pane's agent record, piping the payload to `ngtermctl agent hook codex <event>`, so those three read the payload up
# front and hand it on to the readers below. A subagent's events fire here with its `agent_id` and its own transcript:
# only its permission prompt is reported, and `agent hook` keeps only the prompt of it.
#
# ngterm's agent mail (an ngterm extension): UserPromptSubmit, PostToolUse and Stop also hand the pane's mailbox to
# Codex, so a message another program left with `ngtermctl msg send --to SESSION:PANE` reaches it without anyone typing
# into its composer. `ngtermctl agent mail codex` reads the payload, takes the messages in one read+ack and prints the
# hook's answer, which this prints for Codex: added context on the first two, and on Stop a block whose reason is the
# mail, so the turn goes on (the row stays active then). Only an interactive Codex takes mail, as its rollout's
# session_meta records it (`source` `cli`) and, on Linux and macOS, the argv of the Codex above the hook confirms (a
# `codex exec resume` keeps the TUI session's record): a `codex exec` or app-server run (a worker another agent started
# in this pane inherits its NGTERM_*) takes none, nor does a subagent's event or the memory-consolidation thread. An
# idle Codex has no hook to wake it, so mail sent then waits for its next prompt.
set -u

[ -n "${NGTERM_SESSION_ID:-}" ] || exit 0

action=${1:-}
[ -n "$action" ] || exit 0
shift

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd)
status_wrapper=${NGTERM_STATUS_WRAPPER:-"$script_dir/ngterm-agent-status.sh"}

pane_args=()
[ -n "${NGTERM_PANE_ID:-}" ] && pane_args+=(--pane-id "$NGTERM_PANE_ID")
[ -n "${NGTERM_PANE:-}" ] && pane_args+=(--pane "$NGTERM_PANE")
socket_args=()
[ -n "${NGTERM_SOCKET:-}" ] && socket_args=(--socket "$NGTERM_SOCKET")

report_status() {
  "$status_wrapper" "$@" >/dev/null 2>&1 || true
}

payload=
case "$action" in
  session-start|user-prompt-submit|permission-request)
    payload=$(cat)
    exec <<<"$payload"
    ;;
  post-tool-use|stop)
    # read only where mail can go: a PostToolUse payload holds the whole tool output (the spawner guard below reads
    # it too, on any event, where only the rollout can tell a `codex exec` run)
    if [ -n "${NGTERM_PANE_ID:-}" ]; then
      payload=$(cat)
      exec <<<"$payload"
    fi
    ;;
esac

# ngterm's spawner guard (an ngterm extension: upstream's hook has none). A Codex that another agent started inside a
# pane inherits that pane's NGTERM_*, so its events would paint the spawner's row: the Stop of a reviewer's `codex
# exec` under a Claude Code worker posted `blocked` there for a "?" in the review. Two rules keep such a run silent:
# - a second agent above this hook's Codex and below the pane's origin makes it a worker's. This is the Claude
#   adapter's walk (ngterm-claude-status.sh explains it), with the same agent names and the same pane boundary: keep
#   the two in step;
# - a Codex that runs without its TUI (`codex exec`, `codex review`) has no user at its end, whoever started it. Its
#   argv is read whole where /proc gives it (Linux). Elsewhere (macOS) only the words `ps` joined are there, and a
#   value with a space splits in them, so they decide alone only where a split cannot mislead: no option before the
#   first positional, or `resume`/`fork` as that positional, is the TUI and reports (a resumed session keeps the
#   `source` it was created with, so the rollout cannot say so); `exec resume` with `--last` among its options or a
#   session id as its first positional is headless and silent (it keeps a TUI session's `cli`), and no later word is
#   looked at, since those are the prompt's. Any other words, or no process table at all (Git Bash on Windows), ask
#   the rollout's session_meta how the session was created (`source`: `exec`, or `cli` for the TUI); with no rollout
#   the words stand. What the words still get wrong, all behind an option whose value has a
#   space: a resume in either direction (`codex -C "/my dir" exec resume <TUI session>` reports, `codex -C "/my dir"
#   resume <exec session>` is silent), and a `codex exec` whose split value leaves `resume` or `fork` as the first
#   positional (`codex -C "/my resume" exec`), which reports. So does a `codex exec resume` of a TUI session by a
#   thread name, not an id, or with `--last` behind its prompt: a TUI prompt can start with the same words (`codex
#   "exec resume is failing"`). And the other way round, a TUI session whose prompt starts with `exec resume --last`
#   or `exec resume <session id>` is silent: its words are those of the headless run.
#   `ngtermctl agent mail` also refuses the servers (app-server, mcp-server): they report here, since a TUI on
#   Codex's shared daemon fires its hooks from one.
# Whatever cannot be read means reporting (fail open). The blocker watcher is this hook's own child and is not asked.
is_agent() {
  case "$1" in claude | codex | kimi | opencode | pi | omp | gemini | cursor-agent | aider | crush | goose) return 0 ;; esac
  if [ -n "${NGTERM_AGENT_RE:-}" ]; then
    [[ $1 =~ $NGTERM_AGENT_RE ]] 2>/dev/null && return 0
  fi
  return 1
}

# Reads the Codex argv "$@" into $codex_run. `headless`: it names a subcommand that runs without the TUI, its first
# word that is not an option or an option's value, read as `ngtermctl agent mail` reads it (codex_process.rs has the
# value options every form takes; the ones `exec` alone takes are listed only here, since that reader stops at the
# subcommand); `exec-resume`: that subcommand is `exec`, `resume` is its own first such word, and behind it stands
# `--last` among the options or a session id as the first positional (a later word is the prompt's and says nothing: a
# TUI prompt that starts with `exec resume` may name an id or `--last` further on). Otherwise the TUI: `tui`, or
# `tui?` when options stand before `--` or a first positional other than `resume` or `fork`, where one value with a
# space, split into words, would hide the subcommand behind it.
codex_run=
codex_argv_run() {
  local opts='' in_exec=''
  local id_re='^[0-9a-fA-F]{8}-([0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$'
  codex_run=tui
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      --) # the prompt follows, or behind `exec resume` the session id
        [ -z "$in_exec" ] && [ -n "$opts" ] && codex_run='tui?'
        [ "$in_exec" = resume ] && [ $# -gt 1 ] && [[ $2 =~ $id_re ]] && codex_run=exec-resume
        return
        ;;
      --last)
        opts=1
        if [ "$in_exec" = resume ]; then
          codex_run=exec-resume
          return
        fi
        ;;
      -c | --config | -m | --model | -p | --profile | -C | --cd | -s | --sandbox | -a | --ask-for-approval | \
        --add-dir | --enable | --disable | --local-provider)
        opts=1
        [ $# -gt 1 ] && shift
        ;;
      -o | --output-last-message | --output-schema | --color) # of `exec` alone
        opts=1
        [ -n "$in_exec" ] && [ $# -gt 1 ] && shift
        ;;
      -i | --image)
        opts=1
        while [ $# -gt 1 ] && [ "${2#-}" = "$2" ]; do
          if [ -z "$in_exec" ]; then
            case "$2" in exec | e | review)
              codex_run=headless
              return
              ;;
            esac
          fi
          shift
        done
        ;;
      -*) opts=1 ;;
      *)
        case "$in_exec:$1" in
          resume:*) # exec resume's first positional: the session, by id or by thread name
            [[ $1 =~ $id_re ]] && codex_run=exec-resume
            return
            ;;
          1:resume) in_exec=resume ;;
          1:*) return ;;
          :exec | :e)
            codex_run=headless
            in_exec=1
            ;;
          :review)
            codex_run=headless
            return
            ;;
          :resume | :fork) return ;;
          *)
            [ -n "$opts" ] && codex_run='tui?'
            return
            ;;
        esac
        ;;
    esac
    shift
  done
}

# Prints the `source` of the rollout the payload names, from its first line (the session_meta record); fails when the
# payload names none or it cannot be read. A key quoted inside a string value is escaped, so neither match can hit one.
rollout_source() {
  local path line=''
  local path_re='(^|[^\\])"transcript_path"[[:space:]]*:[[:space:]]*"(([^"\\]|\\.)*)"'
  local source_re='[^\\]"source"[[:space:]]*:[[:space:]]*"([a-z_-]+)"'
  [[ $payload =~ $path_re ]] || return 1
  path=${BASH_REMATCH[2]}
  path=${path//\\\\/\\} # a Windows path's separators
  { IFS= read -r line <"$path" || [ -n "$line" ]; } 2>/dev/null || return 1
  [[ $line =~ $source_re ]] || return 1
  printf '%s' "${BASH_REMATCH[1]}"
}

spawned_run() {
  local agents=0 prev_agent='' p=$PPID line ppid cmd base rest hosted host word
  local codex_pid='' codex_cmd='' codex_host=''
  local proc=${NGTERM_CODEX_PROC:-/proc}
  local -a argv=()
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
    host=
    case "$base" in # node/bun-hosted CLIs present as `node .../cli.js`, so test argv[1] too
      node | bun | deno | python3 | python)
        rest=${cmd#* }
        hosted=${rest%% *}
        hosted=${hosted##*/}
        if is_agent "$hosted"; then
          base=$hosted
          host=1
        fi
        ;;
    esac

    if is_agent "$base"; then
      [ "$base" = "$prev_agent" ] || agents=$((agents + 1))
      prev_agent=$base
      if [ -z "$codex_pid" ] && [ "$base" = codex ]; then # the nearest Codex: the one that fired this hook
        codex_pid=$p
        codex_cmd=$cmd
        codex_host=$host
      fi
    else
      prev_agent=
    fi
    [ "$agents" -gt 1 ] && return 0 # a second agent above mine: I am a worker, not the pane's agent
    # the pane boundary: the app, a headless origin, or the session server that holds a live session's shell. Whatever
    # is above started the origin (an agent's shell that ran ngterm-headless, say) and is not in the pane.
    case "$base" in login | ngterm | ngterm-headless | ngterm-session) break ;; esac
    p=$ppid
  done

  if [ -n "$codex_pid" ] && [ -r "$proc/$codex_pid/cmdline" ]; then
    while IFS= read -r -d '' word; do argv+=("$word"); done <"$proc/$codex_pid/cmdline"
    [ -n "$codex_host" ] && [ "${#argv[@]}" -gt 1 ] && argv=("${argv[@]:1}")
    if [ "${#argv[@]}" -gt 0 ]; then
      codex_argv_run "${argv[@]}"
      [ "$codex_run" = headless ] || [ "$codex_run" = exec-resume ]
      return
    fi
  fi
  # the words `ps` joined: a prompt or an option's value with a space splits, so they can read a TUI as headless
  # (`codex "review this"`) and a headless run as the TUI (`codex -C "/my dir" exec`, whose first positional reads
  # `dir`). They are this run's answer only where no split can mislead (`tui`, `exec-resume`), and there they
  # overrule the rollout: a session keeps the `source` it was created with through a resume.
  codex_run=
  if [ -n "$codex_cmd" ]; then
    read -r -a argv <<<"$codex_cmd"
    [ -n "$codex_host" ] && [ "${#argv[@]}" -gt 1 ] && argv=("${argv[@]:1}")
    codex_argv_run "${argv[@]}"
    case "$codex_run" in
      tui) return 1 ;;
      exec-resume) return 0 ;;
    esac
  fi
  if [ -z "$payload" ] && [ ! -t 0 ]; then
    payload=$(cat)
    exec <<<"$payload"
  fi
  case "$(rollout_source)" in
    exec) return 0 ;;
    cli) return 1 ;;
  esac
  [ "$codex_run" = headless ] # no rollout: the words stand
}

if [ "$action" != __watch-blocked ] && spawned_run; then
  exit 0
fi
export NGTERM_STATUS_WALKED=1 # the status wrapper need not walk again, nor the watcher's, whose chain is gone

# Pipes the payload to `ngtermctl agent hook codex <event>`; a subagent's event other than a permission prompt is left
# out, since its transcript is not the pane's conversation.
report_agent() {
  local subagent_re='(^|[^\\])"agent_id"[[:space:]]*:[[:space:]]*"[^"]'
  if [ "$1" != PermissionRequest ] && [[ $payload =~ $subagent_re ]]; then
    return 0
  fi
  printf '%s' "$payload" | "${NGTERMCTL:-ngtermctl}" agent hook codex "$1" \
    --target "$NGTERM_SESSION_ID" \
    "${socket_args[@]+"${socket_args[@]}"}" \
    "${pane_args[@]+"${pane_args[@]}"}" >/dev/null 2>&1 || true
}

assistant_message_contains_question() {
  local message prose question quoted_mark
  # ngterm's Linux changes to this script: with no plutil, python3 or jq reads the payload's field, and awk, sed,
  # tr and tail are found on PATH (NixOS, Guix and a non-merged /usr have no /usr/bin copies)
  if [ -x /usr/bin/plutil ]; then
    message=$(/usr/bin/plutil -extract last_assistant_message raw -o - - 2>/dev/null) || return 1
  elif command -v python3 >/dev/null 2>&1; then
    message=$(python3 -c '
import json
import sys

value = json.load(sys.stdin).get("last_assistant_message")
if not isinstance(value, str):
    raise SystemExit(1)
sys.stdout.write(value)
' 2>/dev/null) || return 1
  elif command -v jq >/dev/null 2>&1; then
    message=$(jq -er '.last_assistant_message | strings' 2>/dev/null) || return 1
  else
    return 1
  fi
  # a ? that opens quoted text ("? for shortcuts") ends no word, yet its opener would pass as one
  quoted_mark=$'s/(^|[[:space:]])([“‘"\'([*_]+)\\?/\\1\\2/g'
  # a span becomes a word rather than nothing so "run `make test`?" keeps its ? attached
  prose=$(printf '%s\n' "$message" \
    | awk '/^[[:space:]]*```/ { fenced = !fenced; next } !fenced' \
    | sed -E -e 's/`[^`]*`/code/g' -e "$quoted_mark")
  # bash 3.2 cannot parse this bracket set inline in [[ ]]
  question=$'[^[:space:]]\\?[]"\')*_!’”]*([[:space:]]|$)'
  [[ "$prose" =~ $question ]]
}

# whether the payload's transcript_path is there and null. plutil types a null as "(any)"; with no plutil, python3 or
# jq reads it, and with none of the three nothing counts as null, so the event is reported (fail open)
transcript_path_is_null() {
  if [ -x /usr/bin/plutil ]; then
    [ "$(/usr/bin/plutil -type transcript_path - 2>/dev/null)" = "(any)" ]
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c '
import json
import sys

payload = json.load(sys.stdin)
if not (isinstance(payload, dict) and "transcript_path" in payload and payload["transcript_path"] is None):
    raise SystemExit(1)
' 2>/dev/null
  elif command -v jq >/dev/null 2>&1; then
    jq -e 'type == "object" and has("transcript_path") and .transcript_path == null' >/dev/null 2>&1
  else
    return 1
  fi
}

# Takes the pane's mail for this event into $mail_output, the hook answer `ngtermctl agent mail codex` prints; fails
# when there is none or this Codex takes no mail (`agent mail` hands it only to the TUI, as the session_meta of the
# payload's rollout and the Codex process above it say: not to a `codex exec`, app-server or subagent run, nor to the
# memory-consolidation thread, which has no rollout).
mail_output=
take_mail() {
  [ -n "${NGTERM_PANE_ID:-}" ] && [ -n "$payload" ] || return 1
  mail_output=$(printf '%s' "$payload" | "${NGTERMCTL:-ngtermctl}" agent mail codex \
    "${socket_args[@]+"${socket_args[@]}"}" 2>/dev/null) || true
  [ -n "$mail_output" ]
}

read_visible_screen() {
  "${NGTERMCTL:-ngtermctl}" session text \
    --target "$NGTERM_SESSION_ID" \
    "${socket_args[@]+"${socket_args[@]}"}" \
    "${pane_args[@]+"${pane_args[@]}"}" 2>/dev/null
}

watch_file_path() {
  if [ -n "${NGTERM_CODEX_WATCH_FILE:-}" ]; then
    printf '%s' "$NGTERM_CODEX_WATCH_FILE"
    return
  fi
  local key pane_key
  pane_key=${NGTERM_PANE_ID:-${NGTERM_PANE:-left}}
  key=$(printf '%s-%s' "$NGTERM_SESSION_ID" "$pane_key" | tr -c 'A-Za-z0-9._-' '_')
  printf '%s/ngterm-codex-watch-%s-%s' "${TMPDIR:-/tmp}" "${UID:-0}" "$key"
}

stop_watcher() {
  rm -f -- "$(watch_file_path)" >/dev/null 2>&1 || true
}

start_watcher() {
  # Under Git Bash a Codex was seen waiting at "Running hook" while the watcher lived; the cause is not known.
  case "${OSTYPE:-}" in msys*|cygwin*|win32*) return 0 ;; esac
  local token token_file
  token_file=$(watch_file_path)
  token="$$-${RANDOM:-0}"
  (umask 077; printf '%s\n' "$token" > "$token_file") 2>/dev/null || return 0
  "$0" __watch-blocked "$token" "$token_file" </dev/null >/dev/null 2>&1 &
}

watch_for_blocker() {
  local token=${1:-} token_file=${2:-}
  [ -n "$token" ] && [ -n "$token_file" ] || return 0

  local max_checks=${NGTERM_CODEX_WATCH_MAX_CHECKS:-5400}
  local interval=${NGTERM_CODEX_WATCH_INTERVAL:-2}
  case "$max_checks" in ''|*[!0-9]*) max_checks=5400 ;; esac

  local checks=0 prompt_was_visible=0 current_token screen footer
  while [ "$checks" -lt "$max_checks" ]; do
    current_token=$(cat "$token_file" 2>/dev/null) || break
    [ "$current_token" = "$token" ] || break
    screen=$(read_visible_screen) || break
    footer=$(printf '%s\n' "$screen" | tail -n 12 | tr '[:upper:]' '[:lower:]')

    case "$footer" in
      *"press enter to confirm"*|*"enter to submit answer"*|*"enter to submit all"*|*"allow command?"*)
        if [ "$prompt_was_visible" -eq 0 ]; then
          # Re-check the token right before writing: a superseding lifecycle event (stop /
          # pre-tool-use / post-tool-use) may have fired during read_visible_screen, and its
          # status must not be clobbered by a late blocked from this now-stale pane read.
          current_token=$(cat "$token_file" 2>/dev/null) || break
          [ "$current_token" = "$token" ] || break
          report_status blocked
          prompt_was_visible=1
        fi
        ;;
      *)
        # The prompt cleared: restore active so a resolved dialog — especially a denied
        # request that fires no follow-up tool event — doesn't linger blocked until the
        # next lifecycle hook lands.
        [ "$prompt_was_visible" -eq 1 ] && report_status active --blink
        prompt_was_visible=0
        ;;
    esac

    checks=$((checks + 1))
    [ "$checks" -ge "$max_checks" ] && break
    [ "$interval" = "0" ] || sleep "$interval"
  done
}

if [ "$action" = "__watch-blocked" ]; then
  watch_for_blocker "${1:-}" "${2:-}"
  exit 0
fi

case "$action" in
  session-start|user-prompt-submit|pre-tool-use|post-tool-use)
    # codex's memory consolidation thread reaches these hooks but no user Stop handler, so its
    # events would leave the pane active and replace the foreground watcher. it is ephemeral, so its
    # transcript_path is null, which plutil types as "(any)". foreground ephemeral sessions lose
    # these updates too.
    transcript_path_is_null && exit 0
    ;;
esac

case "$action" in
  session-start)
    report_agent SessionStart
    stop_watcher
    report_status idle
    ;;
  user-prompt-submit)
    report_agent UserPromptSubmit
    report_status active --blink
    start_watcher
    take_mail && printf '%s\n' "$mail_output"
    ;;
  pre-tool-use)
    report_status active --blink
    ;;
  post-tool-use)
    report_status active --blink
    take_mail && printf '%s\n' "$mail_output"
    ;;
  permission-request)
    # Candidate only: keep watching while Auto Review decides. The watcher reports blocked only if
    # Codex subsequently leaves a real approval prompt visible for the user.
    report_agent PermissionRequest
    start_watcher
    ;;
  stop)
    if take_mail; then
      printf '%s\n' "$mail_output" # the turn goes on with the mail: the row stays active, the watcher keeps running
      report_status active --blink
      exit 0
    fi
    stop_watcher
    if assistant_message_contains_question; then
      report_status blocked
    else
      report_status completed --auto-reset
    fi
    ;;
esac
exit 0
