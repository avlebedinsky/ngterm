"""Windows: a pane's end ends the programs its shell started, also a hidden one on a console of its own, and nothing
else (capability `windows-shells`).

Each Windows pane child joins a kill-on-close job of its own before it runs (`ngterm-pty`'s `windows` module). A
program that the shell starts outside the pane's console (`Start-Process -WindowStyle Hidden`, `start ""`) does not
end with the console: only the job ends it. The close of one session must not end another session's programs, nor a
program that was not started in a pane (issue 19: the first pane's job once took the shells of the later ones).

The `pwsh-store` case is PowerShell from the Microsoft Store (a packaged app, issue 19). Windows starts the programs of
a packaged shell outside every job, so only the pass that takes them back into the pane's job ends its hidden ping.
That case alone fails when the pass is gone. The other cases are not packaged: their pings stay in the job. CI has no
Store PowerShell, so that case skips there, and its id is not in the Windows ratchet."""

import os
import random
import shutil
import subprocess
import sys

import pytest
from agterm_conformance.match import assert_ok, session_ids
from agterm_conformance.wait import wait_until

WINDOWS = sys.platform == "win32"
if WINDOWS:
    from agterm_conformance import winproc

WINDOWS_SHELLS = pytest.mark.requires(
    "windows-shells", reason="Rust-only, Windows: a pane's tree is a job that its PowerShell or cmd.exe shell is in"
)
# How long a pane's PowerShell may take to start on a loaded Windows: its first start in a fresh profile builds caches.
SHELL_START = 60.0
# How long the end of a pane's tree may take: the job's end, after the pass that takes back what escaped it.
TREE_END = 15.0


def _windows_shell(name):
    """The full path of the Windows shell `name`, or None when this machine has none: `powershell` 5.1 and `cmd` from
    the system directory, `pwsh` 7 from `PATH` but not a packaged one, `pwsh-store` as the Store PowerShell's app
    execution alias."""
    system = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")
    if name == "powershell":
        path = os.path.join(system, "WindowsPowerShell", "v1.0", "powershell.exe")
    elif name == "cmd":
        path = os.path.join(system, "cmd.exe")
    elif name == "pwsh-store":
        path = os.path.join(os.environ.get("LOCALAPPDATA", ""), "Microsoft", "WindowsApps", "pwsh.exe")
    else:
        # `PATH` can give the Store alias first: then this case would be the packaged one, as `pwsh-store`
        folders = os.environ.get("PATH", "").split(os.pathsep)
        unpackaged = os.pathsep.join(f for f in folders if f and "windowsapps" not in os.path.normcase(f))
        path = shutil.which("pwsh", path=unpackaged)
    return path if path and os.path.isfile(path) else None


def _addresses(count):
    """`count` loopback addresses that no other test pings: each ping is found by its address."""
    second, third = random.randrange(1, 255), random.randrange(1, 255)
    return [f"127.{second}.{third}.{last}" for last in range(1, count + 1)]


def _lines(shell, hidden, console):
    """What a user types to start a ping to `hidden` outside the pane's console, then one to `console` on it."""
    if shell == "cmd":
        return [f'start "" /min ping -n 600 {hidden}\n', f"ping -n 600 {console}\n"]
    return [
        f"Start-Process -WindowStyle Hidden ping -ArgumentList '-n','600','{hidden}'\n",
        f"ping -n 600 {console}\n",
    ]


def _pings():
    """The running pings by the address they ping: {address: winproc.Process}."""
    found = {}
    for process in winproc.processes():
        if process.name.lower() != "ping.exe":
            continue
        argv = winproc.command_line(process.pid) or []
        if argv:
            found[argv[-1]] = process
    return found


def _wait_prompt(ctl, sid, shell):
    """Wait for `shell`'s first prompt: what is typed before PowerShell's line editor reads keys can be lost."""
    prompt = r"(?m)^[A-Z]:\\[^\n]*>\s*\Z" if shell == "cmd" else r"(?m)^PS [^\n]*>\s*\Z"
    ctl.wait_text(sid, prompt, timeout=SHELL_START)


def _start_pings(ctl, sid, shell, hidden, console):
    _wait_prompt(ctl, sid, shell)
    first, second = _lines(shell, hidden, console)
    assert_ok(ctl.request("session.type", sid, text=first))
    wait_until(lambda: hidden in _pings(), timeout=SHELL_START)
    assert_ok(ctl.request("session.type", sid, text=second))
    wait_until(lambda: console in _pings(), timeout=SHELL_START)


@WINDOWS_SHELLS
@pytest.mark.covers("session.close")
@pytest.mark.parametrize("shell", ["powershell", "pwsh", "pwsh-store", "cmd"])
def test_a_session_close_ends_its_hidden_programs_and_leaves_the_others(launch, shell):
    path = _windows_shell(shell)
    if path is None:
        pytest.skip(f"no {shell} on this machine")
    ctl = launch(pane_shell=path)
    (first,) = session_ids(ctl.tree())
    ctl.wait_mounted(first, "left")
    second = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(second, "left")
    hidden_a, console_a, hidden_b, console_b, outside = _addresses(5)
    # a program that no pane started, as one from the Start menu
    elsewhere = subprocess.Popen(
        ["ping", "-n", "600", outside], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    try:
        _start_pings(ctl, first, shell, hidden_a, console_a)
        _start_pings(ctl, second, shell, hidden_b, console_b)
        others = {address: _pings()[address] for address in (hidden_b, console_b)}

        assert_ok(ctl.request("session.close", first))

        def first_tree_ended():
            running = _pings()
            assert hidden_a not in running and console_a not in running, sorted(running)
            return True

        wait_until(first_tree_ended, timeout=TREE_END)
        # the other session, its programs and the program outside ngterm run on
        assert session_ids(ctl.tree()) == [second]
        for address, process in others.items():
            assert winproc.alive(process), f"the ping to {address} of the other session ended with the first one"
        assert elsewhere.poll() is None, "a ping that no pane started ended with the first session"

        assert_ok(ctl.request("session.close", second))

        def second_tree_ended():
            running = _pings()
            assert hidden_b not in running and console_b not in running, sorted(running)
            return True

        wait_until(second_tree_ended, timeout=TREE_END)
        assert elsewhere.poll() is None, "a ping that no pane started ended with the last session"
    finally:
        elsewhere.kill()
        elsewhere.wait()
