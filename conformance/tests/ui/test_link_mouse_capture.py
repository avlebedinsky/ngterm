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
if sys.argv[3] == "capture":
    os.write(1, b"\x1b[?1000h\x1b[?1006h")
os.write(1, b"\x1b[2J\x1b[H")
for row in range(rows - 1):
    os.write(1, (link + "L" * (cols - 1) + end + "\r\n").encode())
os.write(1, b"probe-ready")
with open(sys.argv[2], "wb", buffering=0) as output:
    while data := os.read(0, 4096):
        output.write(data)
"""


CTRL, SHIFT = 0xFFE3, 0xFFE1


def _post(ui, *events):
    """XTest input in order: a keysym as an int (press), -keysym (release), "down" or "up" for the left button."""
    from Xlib import X
    from Xlib.ext import xtest

    connection = ui.connection
    for event in events:
        if event in ("down", "up"):
            xtest.fake_input(connection, X.ButtonPress if event == "down" else X.ButtonRelease, 1)
        else:
            kind = X.KeyPress if event > 0 else X.KeyRelease
            xtest.fake_input(connection, kind, connection.keysym_to_keycode(abs(event)))
        connection.sync()


def _click_at_rest(ui, window, shift):
    ui.move(window)
    keys = [CTRL, SHIFT] if shift else [CTRL]
    try:
        _post(ui, *keys, "down", "up")
    finally:
        _post(ui, *(-key for key in reversed(keys)))


def _start_probe(ctl, tmp_path, capture):
    """A session link target, and a probe that fills its pane with links to it (mouse capture on or off) and records
    its input. Returns (target, source, captured file)."""
    target = assert_ok(ctl.request("session.new", noSelect=True))["id"]
    probe = tmp_path / "mouse_probe.py"
    captured = tmp_path / "mouse-input"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe), f"ngterm://session/{target}", os.fspath(captured),
             "capture" if capture else "none"]
    assert all(" " not in word for word in words)
    try:
        source = assert_ok(ctl.request("session.new", command=" ".join(words)))["id"]
    except BaseException:
        ctl.request("session.close", target)
        raise
    return target, source, captured


def _close(ctl, *sessions):
    for session in sessions:
        ctl.request("session.close", session)


def _marked_input(ui, captured, marker):
    """The probe's input up to a typed `marker` key, which proves that every earlier event arrived."""
    ui.press(marker)
    wait_until(lambda: captured.exists() and captured.read_bytes().endswith(marker.encode()))
    return captured.read_bytes()


@pytest.mark.covers("session.new", "session.text", "session.type", "tree")
def test_ctrl_click_reaches_the_program_and_ctrl_shift_opens_the_session_link(ctl, ui, tmp_path):
    target, source, captured = _start_probe(ctl, tmp_path, capture=True)
    try:
        ctl.wait_mounted(source, "left")
        ctl.wait_text(source, "probe-ready")
        window = ui.window()
        _click_at_rest(ui, window, shift=False)
        reports = _marked_input(ui, captured, "x")
        assert re.fullmatch(rb"\x1b\[<16;(\d+);(\d+)M\x1b\[<16;\1;\2mx", reports), reports
        assert find_session(ctl.tree(), source).get("active") is True

        _click_at_rest(ui, window, shift=True)
        ctl.wait_tree(lambda tree: find_session(tree, target).get("active") is True)
        assert_ok(ctl.request("session.type", source, text="y"))
        wait_until(lambda: captured.read_bytes().endswith(b"y"))
        assert captured.read_bytes() == reports + b"y"
    finally:
        _close(ctl, source, target)


@pytest.mark.covers("session.new", "session.text", "tree")
def test_a_shift_press_during_a_program_click_keeps_the_release_with_the_program(ctl, ui, tmp_path):
    """Ctrl, button down (reported), Shift, button up: the program gets the release, and the link takes nothing."""
    target, source, captured = _start_probe(ctl, tmp_path, capture=True)
    try:
        ctl.wait_mounted(source, "left")
        ctl.wait_text(source, "probe-ready")
        ui.move(ui.window())
        try:
            _post(ui, CTRL, "down", SHIFT, "up")
        finally:
            _post(ui, -SHIFT, -CTRL)
        reports = _marked_input(ui, captured, "x")
        assert re.fullmatch(rb"\x1b\[<16;(\d+);(\d+)M\x1b\[<20;\1;\2mx", reports), reports
        tree = ctl.tree()
        assert find_session(tree, source).get("active") is True
        assert find_session(tree, target).get("active") is not True
    finally:
        _close(ctl, source, target)


@pytest.mark.covers("session.new", "session.text")
def test_a_shift_release_during_a_local_click_sends_the_program_no_report(ctl, ui, tmp_path):
    """Shift (a local selection) or Ctrl+Shift (the link), button down, Shift up, button up: the press stayed local,
    so the program gets no unmatched release."""
    target, source, captured = _start_probe(ctl, tmp_path, capture=True)
    try:
        ctl.wait_mounted(source, "left")
        ctl.wait_text(source, "probe-ready")
        ui.move(ui.window())
        try:
            _post(ui, SHIFT, "down", -SHIFT, "up")
            _post(ui, CTRL, SHIFT, "down", -SHIFT, "up")
        finally:
            _post(ui, -SHIFT, -CTRL)
        assert _marked_input(ui, captured, "x") == b"x"
    finally:
        _close(ctl, source, target)


@pytest.mark.covers("session.new", "session.text", "tree")
def test_ctrl_click_opens_the_session_link_after_a_drag_selection_without_capture(ctl, ui, tmp_path):
    """No mouse capture: a drag over the links selects text, and the next Ctrl-click at rest still opens a link."""
    target, source, captured = _start_probe(ctl, tmp_path, capture=False)
    try:
        ctl.wait_mounted(source, "left")
        ctl.wait_text(source, "probe-ready")
        window = ui.window()
        ui.drag_by(window, 60, 40, offset=(0.5, 0.4), steps=6, duration=0.3)
        assert _marked_input(ui, captured, "x") == b"x"
        assert find_session(ctl.tree(), target).get("active") is not True
        _click_at_rest(ui, window, shift=False)
        ctl.wait_tree(lambda tree: find_session(tree, target).get("active") is True)
        assert captured.read_bytes() == b"x"
    finally:
        _close(ctl, source, target)
