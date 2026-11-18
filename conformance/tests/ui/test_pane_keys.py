"""Keys pressed into a pane whose program pushed the Kitty keyboard flags (`CSI > 1 u`) and `modifyOtherKeys` 2
(`CSI > 4 ; 2 m`), as Claude Code does: Escape, Shift+Tab, Ctrl+C and Alt+V each reach the program.

The pane's program is a probe, the harness's own Python, that pushes both modes, prints `keys-ready` and then a
`key <name> <form>` line for each key it reads. On Linux and macOS it reads the pane's bytes in raw mode: the form is
`kitty` for a Kitty `CSI u` encoding and `legacy` for the legacy one. The terminal sends what the program asked for, the
Kitty encodings (Ghostty's key encoder, upstream's too). On Windows the pseudoconsole stands between the pane and the
program: it reads the pane's bytes and gives the program console key events (`ReadConsoleInputW`, the form `console`),
and its input parser drops the Kitty and `modifyOtherKeys` encodings, so there a pane always sends the legacy ones
(`PaneConfig::legacy_keys`). The Mac's Option is a text key by default (Ghostty's `macos-option-as-alt`), so Alt+V
is not pressed there. The command line is the probe's words alone, unquoted, as `kitty_clipboard.probe_command`'s."""

import os
import re
import sys

import pytest
from agterm_conformance.match import assert_ok

WINDOWS = sys.platform == "win32"
CHORDS = ("escape", "shift+tab", "ctrl+c") + (() if sys.platform == "darwin" else ("alt+v",))
PROBE = r"""
import os, re

PUSH = b"\x1b[>1u\x1b[>4;2m"


def say(text):
    os.write(1, (text + "\r\n").encode())


def name(key, ctrl, alt, shift):
    return "".join(("ctrl+" if ctrl else "", "alt+" if alt else "", "shift+" if shift else "", key))


if os.name == "nt":
    import ctypes
    from ctypes import wintypes

    class Key(ctypes.Structure):
        _fields_ = [("down", wintypes.BOOL), ("repeat", wintypes.WORD), ("vk", wintypes.WORD),
                    ("scan", wintypes.WORD), ("char", wintypes.WCHAR), ("state", wintypes.DWORD)]

    class Record(ctypes.Structure):  # INPUT_RECORD: its union is no larger than KEY_EVENT_RECORD
        _fields_ = [("type", wintypes.WORD), ("key", Key)]

    kernel32 = ctypes.windll.kernel32
    out, inp = kernel32.GetStdHandle(-11), kernel32.GetStdHandle(-10)
    mode = wintypes.DWORD()
    if kernel32.GetConsoleMode(out, ctypes.byref(mode)):
        kernel32.SetConsoleMode(out, mode.value | 0x1 | 0x4)
    # no line input, no echo, no processed input: Ctrl+C is a key event, not a signal
    kernel32.SetConsoleMode(inp, 0)
    os.write(1, PUSH)
    say("keys-ready")
    record, count = Record(), wintypes.DWORD()
    keys = {0x1B: "escape", 0x09: "tab"}
    while kernel32.ReadConsoleInputW(inp, ctypes.byref(record), 1, ctypes.byref(count)):
        key = record.key
        # key events only, their presses, and not the modifier keys themselves (Shift, Ctrl, Alt, Windows)
        if record.type != 1 or not key.down or key.vk in (0x10, 0x11, 0x12, 0x5B, 0x5C):
            continue
        label = keys.get(key.vk, chr(key.vk).lower() if 0x30 <= key.vk <= 0x5A else f"vk{key.vk:02x}")
        state = key.state
        say(f"key {name(label, state & 0xC, state & 0x3, state & 0x10)} console")
else:
    import tty

    tty.setraw(0)
    os.write(1, PUSH)
    say("keys-ready")
    legacy = {b"\x1b": "escape", b"\x1b[Z": "shift+tab", b"\x03": "ctrl+c", b"\x1bv": "alt+v"}
    while data := os.read(0, 64):
        if data in legacy:
            say(f"key {legacy[data]} legacy")
        elif match := re.fullmatch(rb"\x1b\[(\d+)(?:;(\d+))?u", data):
            code, mods = int(match[1]), int(match[2] or 1) - 1
            label = {27: "escape", 9: "tab"}.get(code, chr(code))
            say(f"key {name(label, mods & 4, mods & 2, mods & 1)} kitty")
        else:
            say(f"key unknown {data.hex()}")
"""


@pytest.mark.covers("behavior:pane.typing")
def test_keys_reach_a_program_that_pushed_the_kitty_keyboard_flags(ctl, ui, tmp_path):
    probe = tmp_path / "keys_probe.py"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe)]
    if any(" " in word for word in words):
        pytest.fail(f"the probe's words go unquoted to every pane shell, so none may hold a space: {words}")
    sid = assert_ok(ctl.request("session.new", command=" ".join(words)))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        ctl.wait_text(sid, "keys-ready")
        forms = {}
        for chord in CHORDS:
            ui.press(chord)
            forms[chord] = ctl.wait_text(sid, rf"key {re.escape(chord)} (\w+)")[1]
        expected = "console" if WINDOWS else "kitty"
        assert forms == dict.fromkeys(CHORDS, expected), assert_ok(ctl.request("session.text", sid, all=True))["text"]
    finally:
        ctl.request("session.close", sid)
