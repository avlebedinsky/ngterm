"""The Linux installers and payload hooks use private homes and the real app."""

import json
import os
import shutil
import stat
import sys
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Cli, CliProcess
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.skipif(not sys.platform.startswith("linux"), reason="Linux payload readers and POSIX file modes"),
    pytest.mark.requires("agent-chat", reason="the installed ngterm hooks"),
]


def _run(cli, argv, env, cwd, input=None):
    process = CliProcess(argv, env, cwd, input, False, cli.program)
    try:
        result = process.finish(timeout=30)
    finally:
        process.close()
    assert result.returncode == 0, result
    return result.stdout


def _evidence(name, text=None, ui=None):
    root = os.environ.get("QA_EVIDENCE_DIR")
    if root:
        path = Path(root)
        path.mkdir(parents=True, exist_ok=True)
        if text is not None:
            (path / f"{name}.txt").write_text(text, encoding="utf-8")
        if ui is not None:
            (path / f"{name}.png").write_bytes(ui.screenshot())


def _prepare(env, case):
    home = Path(env["HOME"])
    home.mkdir(parents=True, exist_ok=True)
    env.pop("CODEX_HOME", None)
    env["SHELL"] = "/bin/bash"
    custom = home / "custom codex"
    default = home / ".codex"
    default.mkdir(exist_ok=True)
    (default / "config.toml").write_text('model = "user-default"\n', encoding="utf-8")
    if case == "empty":
        env["CODEX_HOME"] = ""
        root = default
    else:
        root = custom
        if case == "relative":
            env["CODEX_HOME"] = "custom codex"
        elif case == "login":
            (home / ".bash_profile").write_text('export CODEX_HOME="$HOME/custom codex"\n', encoding="utf-8")
        else:
            env["CODEX_HOME"] = str(custom)
    if case != "missing":
        root.mkdir(exist_ok=True)
        (root / "config.toml").write_text('model = "user-model"\n', encoding="utf-8")
    for rc in (".bashrc", ".zshrc", ".config/fish/config.fish"):
        path = home / rc
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("# user config\n", encoding="utf-8")
        path.chmod(0o600)
    if case == "user-skill":
        skill = root / "skills/ngterm/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("user skill\n", encoding="utf-8")
    return root


@pytest.mark.parametrize("installer", ["help", "headless"])
@pytest.mark.parametrize("case", ["custom", "relative", "empty", "login", "missing", "user-skill"])
def test_agent_installers_keep_the_selected_root_and_file_modes(adapter, monkeypatch, launch, ui_for, installer, case):
    if adapter.name != "rust":
        pytest.skip("the ngterm installers")
    build_env = adapter.build_env
    executable = adapter.instance_executable
    environment = {}

    def configured(root, app_id=None):
        env = build_env(root, app_id)
        env.pop("CODEX_HOME", None)
        environment["env"] = env
        return env

    def prepared(root, private_install=False):
        command = executable(root, private_install)
        _prepare(environment["env"], case)
        return command

    monkeypatch.setattr(adapter, "build_env", configured)
    monkeypatch.setattr(adapter, "instance_executable", prepared)
    ctl = launch(private_install=True)
    cli = Cli(adapter.ctl_path, ctl.instance, adapter.ctl_name)
    ui = ui_for(ctl)
    home = ctl.instance.home
    root = home / (".codex" if case == "empty" else "custom codex")
    install = ctl.instance.root / "install"
    package = install / "share/ngterm/agent-status"
    if package.is_symlink():
        source = package.resolve()
        package.unlink()
        shutil.copytree(source, package)
    headless = install / "bin/ngterm-headless"
    if headless.is_symlink():
        source = headless.resolve()
        headless.unlink()
        shutil.copy2(source, headless)
    modes = {"ngterm-agent-status.sh": 0o700, "ngterm-codex-status.sh": 0o750}
    for name, mode in modes.items():
        (package / name).chmod(mode)
    for repeat in range(2):
        if installer == "headless":
            report = _run(cli, [str(install / "bin/ngterm-headless"), "--install-agent-hooks", "--opencode-version", "v2"],
                          ctl.instance.env, home)
        else:
            reports = []
            for item in ("Install Agent Skill…", "Install Agent Status Hooks…"):
                ui.menu("Help", item)
                if "Hooks" in item:
                    wait_until(lambda: "button name='OK'" in ui.tree_dump() or "Skip OpenCode" in ui.tree_dump())
                    if "Skip OpenCode" in ui.tree_dump():
                        ui.press("return")
                wait_until(lambda: "button name='OK'" in ui.tree_dump())
                reports.append(ui.tree_dump() + "\n" + ctl.instance.log.read_text(encoding="utf-8"))
                _evidence(f"{installer}-{case}-{repeat}-{len(reports)}", reports[-1], ui)
                ui.press("return")
                wait_until(lambda: "button name='OK'" not in ui.tree_dump())
            report = "\n".join(reports)
        _evidence(f"{installer}-{case}-{repeat}", report)
        if case == "missing":
            assert str(root) in report and "no " in report.lower(), report
            assert not root.exists()
        else:
            config = (root / "config.toml").read_text(encoding="utf-8")
            assert 'model = "user-model"' in config and "ngterm-codex-status.sh" in config
            assert (root / "config.toml.bak").read_text(encoding="utf-8") == 'model = "user-model"\n'
            skill = (root / "skills/ngterm/SKILL.md").read_text(encoding="utf-8")
            if case == "user-skill":
                assert skill == "user skill\n"
            else:
                assert "<!-- ngterm-skill -->" in skill
        if case != "empty":
            assert (home / ".codex/config.toml").read_text(encoding="utf-8") == 'model = "user-default"\n'
            assert not (home / ".codex/config.toml.bak").exists()
            assert not (home / ".codex/skills").exists()
        for name, mode in modes.items():
            path = home / ".config/ngterm/agent-status" / name
            assert stat.S_IMODE(path.stat().st_mode) == mode
        for rc in (".bashrc", ".zshrc", ".config/fish/config.fish"):
            path = home / rc
            assert stat.S_IMODE(path.stat().st_mode) == 0o600
            assert path.read_text(encoding="utf-8").startswith("# user config\n")


@pytest.mark.parametrize("parser", ["python3", "jq"])
@pytest.mark.parametrize("agent", ["claude", "codex"])
def test_agent_payloads_reach_the_tree_and_sidebar(cli, ctl, ui, tmp_path, parser, agent):
    if not shutil.which(parser):
        pytest.skip(f"{parser} is absent")
    home = tmp_path / "home"
    env = dict(cli.instance.env, HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"), CODEX_HOME="")
    _prepare(env, "empty")
    (home / ".claude").mkdir(parents=True)
    installer = Path(cli.ctl_path).with_name("ngterm-headless")
    _run(cli, [str(installer), "--install-agent-hooks", "--opencode-version", "v2"], env, home)
    tools = tmp_path / "bin"
    tools.mkdir()
    for name in ("bash", "sh", "dirname", "cat", "awk", "sed", "tr", "tail", "rm", parser):
        executable = shutil.which(name)
        assert executable, name
        (tools / name).symlink_to(executable)
    # The hooks walk their ancestors with ps and stay silent under a second agent. Here the ancestors are the runner
    # of the suite, so a ps that fails stops the walk at once, as crates/ngterm-ctl/tests/agent_hook.rs does.
    (tools / "ps").write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
    (tools / "ps").chmod(0o755)
    sid = assert_ok(ctl.request("session.new"))["id"]
    try:
        ctl.wait_mounted(sid, "left")
        script = home / f".config/ngterm/agent-status/ngterm-{agent}-status.sh"
        pane_id = next(p["paneID"] for p in find_session(ctl.tree(), sid)["surfaces"] if p["kind"] == "left")

        def step(label, args, payload, expected, blink=False, seed="blocked"):
            assert_ok(ctl.request("session.status", sid, status=seed))
            hook_env = dict(env, PATH=str(tools), NGTERMCTL=str(cli.ctl_path), NGTERM_SESSION_ID=sid,
                            NGTERM_PANE="left", NGTERM_PANE_ID=pane_id,
                            NGTERM_SOCKET=str(cli.instance.socket), NGTERM_CODEX_WATCH_MAX_CHECKS="0")
            _run(cli, [str(script), *args], hook_env, home, input=payload)
            node = find_session(ctl.tree(), sid)
            assert node.get("status") == expected, (label, node)
            assert bool(node.get("statusBlink")) == blink, (label, node)
            ui.find("agent-status", value=expected)
            tree = cli("tree", "--json")
            assert tree.returncode == 0, tree
            _evidence(f"{agent}-{parser}-{label}", tree.stdout + "\n" + ui.tree_dump(), ui)

        if agent == "claude":
            subagent = json.dumps({"session_id": "s", "hook_event_name": "PostToolUse", "agent_id": "a1"})
            for seed in ("blocked", "completed"):
                step(f"subagent-{seed}", ["active", "--blink"], subagent, seed, seed=seed)
            step("subagent-idle", ["active", "--blink"], subagent, "active", True, "idle")
            for kind in ("subagent", "workflow", "shell"):
                payload = json.dumps({"session_id": "s", "hook_event_name": "Stop", "stop_hook_active": False,
                                      "background_tasks": [{"type": kind, "status": "running"}]})
                step(f"stop-{kind}", ["completed", "--auto-reset", "--color", "#ff8800"], payload, "active", True)
                assert "statusColor" not in find_session(ctl.tree(), sid)
            for label, payload in (("missing", "{}"), ("unreadable", "not json")):
                step(label, ["completed", "--blink"], payload, "completed", True)
        else:
            for label, payload, expected in (
                ("question", '{"hook_event_name":"Stop","last_assistant_message":"Continue?"}', "blocked"),
                ("missing", "{}", "completed"),
                ("unreadable", "not json", "completed"),
            ):
                step(label, ["stop"], payload, expected)
            step("null-transcript", ["post-tool-use"], '{"transcript_path":null}', "blocked")
            step("missing-transcript", ["post-tool-use"], "{}", "active", True)
    finally:
        assert_ok(ctl.request("session.close", sid))
