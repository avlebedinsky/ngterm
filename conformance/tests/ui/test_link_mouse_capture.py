"""Linux link clicks while the program captures the mouse, as Ghostty handles them."""

import os
import re
import sys

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.skipif(sys.platform != "linux", reason="the test uses XTest on the private Xvfb"),
    pytest.mark.requires("posix-command-line", reason="the probe uses a POSIX terminal"),
]

PROBE = r"""
import os, sys, tty

tty.setraw(0)
cols, rows = os.get_terminal_size()
link = "\x1b]8;;" + sys.argv[1] + "\x1b\\"
end = "\x1b]8;;\x1b\\"
os.write(1, b"\x1b[?1000h\x1b[?1006h\x1b[2J\x1b[H")
for row in range(rows - 1):
    os.write(1, (link + "L" * (cols - 1) + end + "\r\n").encode())
os.write(1, b"capture-ready")
with open(sys.argv[2], "wb", buffering=0) as output:
    while data := os.read(0, 4096):
        output.write(data)
"""


def _click_at_rest(ui, window, shift):
    from Xlib import X
    from Xlib.ext import xtest

    ui.move(window)
    connection = ui.connection
    keys = [connection.keysym_to_keycode(0xffe3)]
    if shift:
        keys.append(connection.keysym_to_keycode(0xffe1))
    try:
        for key in keys:
            xtest.fake_input(connection, X.KeyPress, key)
        connection.sync()
        xtest.fake_input(connection, X.ButtonPress, 1)
        xtest.fake_input(connection, X.ButtonRelease, 1)
        connection.sync()
    finally:
        for key in reversed(keys):
            xtest.fake_input(connection, X.KeyRelease, key)
        connection.sync()


@pytest.mark.covers("session.new", "session.text", "session.type", "tree")
def test_ctrl_click_reaches_the_program_and_ctrl_shift_opens_the_session_link(ctl, ui, tmp_path):
    target = assert_ok(ctl.request("session.new", noSelect=True))["id"]
    source = None
    probe = tmp_path / "mouse_probe.py"
    captured = tmp_path / "mouse-input"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe), f"ngterm://session/{target}", os.fspath(captured)]
    assert all(" " not in word for word in words)
    try:
        source = assert_ok(ctl.request("session.new", command=" ".join(words)))["id"]
        ctl.wait_mounted(source, "left")
        ctl.wait_text(source, "capture-ready")
        window = ui.window()
        _click_at_rest(ui, window, shift=False)
        ui.press("x")
        wait_until(lambda: captured.exists() and captured.read_bytes().endswith(b"x"))
        reports = captured.read_bytes()
        assert re.fullmatch(rb"\x1b\[<16;(\d+);(\d+)M\x1b\[<16;\1;\2mx", reports), reports
        assert find_session(ctl.tree(), source).get("active") is True

        _click_at_rest(ui, window, shift=True)
        ctl.wait_tree(lambda tree: find_session(tree, target).get("active") is True)
        assert_ok(ctl.request("session.type", source, text="y"))
        wait_until(lambda: captured.read_bytes().endswith(b"y"))
        assert captured.read_bytes() == reports + b"y"
    finally:
        if source is not None:
            ctl.request("session.close", source)
        ctl.request("session.close", target)
