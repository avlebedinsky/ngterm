"""An agent that another agent started in a pane leaves that pane's status alone: a Rust-only extension (capability
`agent-chat`).

An agent's shell commands inherit the pane's `NGTERM_*`, so a `codex exec` that Claude Code runs, or a Codex TUI or
an Oh My Pi under it, fires its hooks for the pane of the agent that started it: before the spawner guard, a
reviewer's last message with a question mark set that pane `blocked`. The installed Codex hook now stays silent for a
Codex with a second agent above it below the pane's boundary, and for a Codex that runs without its TUI; the status
wrapper, which the Pi, Oh My Pi and OpenCode extensions call, stays silent for a caller with a second agent above it.
An agent that is the pane's own reports as before.

The hooks are the installed ones: the staged `ngterm-headless --install-agent-hooks` installs them into a home of the
test's own. Each runs in a real pane, so the hook's ancestor walk meets the instance's own processes at the pane's
boundary. The agents are stand-ins: `claude`, `codex` and `omp` are bash under those names (the walk reads each
ancestor's argv[0]). The script a stand-in Codex runs fires the hooks by the commands of its `config.toml` entries, as
Codex does, a Stop payload whose last message asks a question; the one a stand-in Oh My Pi runs calls the wrapper at
the path its extension calls, as at a turn's end."""

import json
import shutil
import tomllib
import uuid
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import CliProcess
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires(
        "agent-chat",
        reason="a Rust-only extension: the spawner guard of the installed Codex hook (upstream's has none)",
    ),
    pytest.mark.requires("posix-shell", reason="the pane's /bin/sh runs the stand-in agents' chain"),
    pytest.mark.requires(
        "posix-command-line",
        reason="the hooks run as POSIX lines, and their ancestor walk reads `ps -o ppid=,command=`, missing on Windows",
    ),
]

HOOK_TIMEOUT = 60.0
EVENTS = ("SessionStart", "UserPromptSubmit", "Stop")
# The script the stand-in Codex runs (as bash's script operand, from the names' directory): its name is the run's
# kind, `exec` for `codex exec`, `hello` for a TUI given a prompt.
RUNNER = """kind=${0##*/}
for event in SessionStart UserPromptSubmit Stop; do
  sh -c "$(cat "$NAMES/$event.cmd")" <"$NAMES/$kind.$event.json" >/dev/null 2>&1
done
: >"$DONE"
"""
# The script the stand-in Oh My Pi runs: its extension's report at a turn's end.
REPORT = """"$HOME_DIR/.config/ngterm/agent-status/ngterm-agent-status.sh" completed --auto-reset >/dev/null 2>&1
: >"$DONE"
"""
# `claude` runs the rest of its line as its child; the trailing `:` keeps bash from replacing itself with it.
UNDER = '"$NAMES/$1" "${@:2}"\n:\n'


def _codex_hook_commands(cli, home: Path) -> dict[str, str]:
    """Installs the build's hooks into `home`; the command Codex runs on each of `EVENTS`."""
    (home / ".codex").mkdir(parents=True)
    (home / ".codex" / "config.toml").write_text("", encoding="utf-8")
    ctl = Path(cli.ctl_path)
    installer = ctl.with_name("ngterm-headless" + ctl.suffix)
    assert installer.is_file(), f"the stage holds no {installer.name} beside {ctl.name}"
    env = dict(cli.instance.env, HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"))
    process = CliProcess([str(installer), "--install-agent-hooks"], env, cli.instance.home, None, False, cli.program)
    try:
        done = process.finish(timeout=HOOK_TIMEOUT)
    finally:
        process.close()
    assert done.returncode == 0 and "Codex lifecycle hooks merged" in done.stdout, done
    hooks = tomllib.loads((home / ".codex" / "config.toml").read_text(encoding="utf-8"))["hooks"]
    commands = {}
    for event in EVENTS:
        found = [hook["command"] for group in hooks[event] for hook in group["hooks"]]
        assert len(found) == 1 and "ngterm-codex-status" in found[0], (event, found)
        commands[event] = found[0]
    return commands


def _names(root: Path, commands: dict[str, str]) -> Path:
    """The stand-ins' directory: `claude`, `codex` and `omp` (bash under those names), the scripts they run, the hook
    commands and the payloads of each kind of run, with the rollout each names."""
    names = root / "names"
    names.mkdir()
    bash = shutil.which("bash")
    assert bash, "no bash on PATH"
    for name in ("claude", "codex", "omp"):
        (names / name).symlink_to(bash)
    for script, text in (("exec", RUNNER), ("hello", RUNNER), ("under", UNDER), ("report", REPORT)):
        (names / script).write_text(text, encoding="utf-8")
    for event, command in commands.items():
        (names / f"{event}.cmd").write_text(command, encoding="utf-8")
    for kind, source in (("exec", "exec"), ("hello", "cli")):
        conversation = str(uuid.uuid4())
        rollout = names / f"rollout-{kind}.jsonl"
        meta = {"type": "session_meta", "payload": {"id": conversation, "cwd": str(root), "source": source}}
        rollout.write_text(json.dumps(meta) + "\n", encoding="utf-8")
        base = {"session_id": conversation, "transcript_path": str(rollout), "cwd": str(root), "model": "gpt-5"}
        payloads = {
            "SessionStart": {"hook_event_name": "SessionStart", "source": "startup"},
            "UserPromptSubmit": {"hook_event_name": "UserPromptSubmit", "turn_id": "1", "prompt": "Ask me."},
            "Stop": {
                "hook_event_name": "Stop", "turn_id": "1", "stop_hook_active": False,
                "last_assistant_message": "Which color do you like best?",
            },
        }
        for event, fields in payloads.items():
            (names / f"{kind}.{event}.json").write_text(json.dumps({**base, **fields}), encoding="utf-8")
    return names


@pytest.fixture
def names(cli, tmp_path):
    return _names(tmp_path, _codex_hook_commands(cli, tmp_path / "home"))


@pytest.mark.parametrize(
    ("chain", "status"),
    [
        pytest.param("codex hello", "blocked", id="a-codex-tui-the-panes-own-agent-reports"),
        pytest.param("claude under codex exec", "active", id="a-codex-exec-claude-started-is-silent"),
        pytest.param("claude under codex hello", "active", id="a-codex-tui-claude-started-is-silent"),
        pytest.param("codex exec", "active", id="a-codex-exec-alone-is-silent"),
        pytest.param("omp report", "completed", id="an-omp-the-panes-own-agent-reports"),
        pytest.param("claude under omp report", "active", id="an-omp-claude-started-is-silent"),
    ],
)
def test_an_agent_another_agent_started_leaves_the_panes_status_alone(cli, ctl, tmp_path, names, chain, status):
    sid = assert_ok(ctl.request("session.new"))["id"]
    try:
        ctl.wait_mounted(sid, "left")
        # the pane's own agent has reported its turn, as Claude Code's hook does
        assert_ok(ctl.request("session.status", sid, status="active"))
        done = names / f"done-{uuid.uuid4().hex}"
        agent, *words = chain.split()
        # bash finds its script operand (`under`, `exec`, `hello`, `report`) in the working directory
        line = (
            f"cd '{names}' && NAMES='{names}' HOME_DIR='{tmp_path / 'home'}' DONE='{done}' "
            f"'{names / agent}' {' '.join(words)}\n"
        )
        assert_ok(ctl.request("session.type", sid, text=line))
        wait_until(done.exists, timeout=HOOK_TIMEOUT)
        shown = find_session(ctl.tree(), sid).get("status")
        assert shown == status, (chain, shown)
    finally:
        assert_ok(ctl.request("session.close", sid))
