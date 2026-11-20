"""Typing into the selected pane through the OS's text input, read back from the terminal: its text and its cursor.

Upstream's UI tests type into the focused terminal with `typeText` (`SessionNavUITests`, `ControlSurfaceZoomUITests`
v0.39.1). The pane runs `cat` instead of a login shell (`session.new --command`), so no prompt of unknown length sits
before the typed text: the terminal echoes it from column 0, and the cursor's column (the protocol reports no row)
counts what was typed without wrapping. The pane mounts before its program starts, and the reference's `login` prints
its `Last login` line before running the command, into the middle of anything typed that early: the test first has
`cat` print a line back, the same probe `test_terminal_bindings` uses.

On the Mac, Option is a text key by default (Ghostty's `macos-option-as-alt`): it types the layout's character. With
the option on, Option is Alt: the key goes with ESC before it, which `cat -v` shows as `^[`. The Option cases press the
US layout's keys (macOS CI's and the test Macs'). Its Option dead keys (e, u, n) compose with the next key, as posted
key codes too (remote desktops, automation tools), and leave no accent for a later key or another session. Backspace
or Escape right after a dead key cancels only its accent: the program gets neither key."""

import contextlib
import re
import sys

import pytest
from agterm_conformance.ghostty_config import configured
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_cursor

MAC = pytest.mark.skipif(sys.platform != "darwin", reason="the Mac's Option key")

# Case, digits and shifted symbols; no Return follows them, so `cat` never prints them back and the echo is their only
# copy in the pane.
FIRST = "Typed"
REST = "-Ab_C:1@2"


@pytest.mark.covers("session.text", "surface.cursor", "behavior:pane.typing")
def test_typed_text_reaches_the_selected_pane_and_moves_its_cursor(ctl, ui):
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        assert_ok(ctl.request("session.type", sid, text="cat-reads\n"))
        ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True)
        # no target: the active surface, the selected session's focused pane the keys go to
        wait_cursor(ctl, None, 0)
        ui.type_text(FIRST)
        ctl.wait_text(sid, re.escape(FIRST))
        wait_cursor(ctl, None, len(FIRST))
        ui.type_text(REST)
        ctl.wait_text(sid, re.escape(FIRST + REST))
        wait_cursor(ctl, None, len(FIRST + REST))
    finally:
        ctl.request("session.close", sid)


# Text expanders and password managers type text as key events that carry it as a Unicode string on a key that does
# not type it (macOS: key code 0, the A key; Windows: VK_PACKET): letters of other scripts, CJK and an emoji.
UNICODE_TEXT = "über-жук-你好-🙂"


@pytest.mark.covers("session.text", "behavior:pane.typing")
def test_text_typed_as_unicode_key_events_reaches_the_pane_as_typed(ctl, ui):
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        assert_ok(ctl.request("session.type", sid, text="cat-reads\n"))
        ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True)
        ui.type_text(UNICODE_TEXT)
        ctl.wait_text(sid, re.escape(UNICODE_TEXT))
    finally:
        ctl.request("session.close", sid)


@contextlib.contextmanager
def _reading(ctl, command):
    """A new session that runs `command`, once it has printed a typed line back."""
    sid = assert_ok(ctl.request("session.new", command=command))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        assert_ok(ctl.request("session.type", sid, text="cat-reads\n"))
        ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True)
        yield sid
    finally:
        ctl.request("session.close", sid)


@MAC
@pytest.mark.covers("session.text", "behavior:pane.typing")
def test_option_types_the_layouts_character(ctl, ui):
    with _reading(ctl, "cat") as sid:
        ui.press("alt+s", "alt+g", "alt+p", "alt+shift+a", "return")
        ctl.wait_text(sid, r"ß©πÅ\s+ß©πÅ", all=True)


@MAC
@pytest.mark.covers("session.text", "behavior:pane.typing")
def test_option_dead_keys_compose_with_the_next_key_and_leave_nothing_pending(ctl, ui):
    with _reading(ctl, "cat") as sid:
        ui.press("alt+e", "e", "alt+u", "u", "alt+n", "n", "alt+u", "a", "1", "alt+e", "space", "2", "return")
        ctl.wait_text(sid, r"éüñä1´2\s+éüñä1´2", all=True)
    with _reading(ctl, "cat") as sid:
        ui.press("3", "alt+u", "u", "alt+s", "return")
        ctl.wait_text(sid, r"\n3üß\s+3üß", all=True)


@MAC
@pytest.mark.covers("session.text", "behavior:pane.typing")
def test_backspace_or_escape_after_a_dead_key_cancels_only_its_accent(ctl, ui):
    with _reading(ctl, "cat -v") as sid:
        ui.press("x", "alt+e", "delete", "e", "alt+u", "escape", "u", "return")
        ctl.wait_text(sid, r"\nxeu\s+xeu", all=True)


@MAC
@pytest.mark.covers("session.text", "config.reload", "behavior:pane.typing")
def test_option_as_alt_sends_escape_before_the_key(ctl, ui):
    with configured(ctl, "macos-option-as-alt = true\n"), _reading(ctl, "cat -v") as sid:
        ui.press("alt+u", "a", "alt+e", "e", "return")
        ctl.wait_text(sid, r"\^\[ua\^\[ee\s+\^\[ua\^\[ee", all=True)
