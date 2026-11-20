"""The quick terminal's panel on macOS, as the user meets it. The in-app chord (ctrl+`, through the OS's keyboard
input) shows it without a title bar: its AX window has none of the title bar's buttons, also when it is zoomed. It
floats over normal windows, and it takes the keyboard: the keys typed next go to its terminal. Upstream's
`QuickTerminalPanel` is `[.borderless, .nonactivatingPanel]` at the floating level and can become key."""

import re
import sys

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="the panel's title bar buttons as macOS AX has them")

BUTTONS = ("AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton")
MAX_DEPTH = 40


def _holds_quick(ui, handle, depth=0):
    if str(ui.attribute(handle, "AXIdentifier") or "") == "quick-terminal":
        return True
    return depth < MAX_DEPTH and any(
        _holds_quick(ui, child, depth + 1) for child in ui.attribute(handle, "AXChildren") or ()
    )


def _size(ui, window):
    ok, size = ui.ax.AXValueGetValue(ui.attribute(window, "AXSize"), ui.ax.kAXValueCGSizeType, None)
    assert ok, "the panel's AX window has no size"
    return float(size.width), float(size.height)


def _quick_windows(ui):
    """The size and the title bar buttons of each AX window that holds the quick terminal."""
    return [
        (_size(ui, window), [name for name in BUTTONS if ui.attribute(window, name) is not None])
        for window in ui.attribute(ui.app, "AXWindows") or ()
        if _holds_quick(ui, window)
    ]


def _layers(ui, pid, size):
    """The window server layers of the on-screen windows of `pid` that have `size`."""
    q = ui.quartz
    windows = q.CGWindowListCopyWindowInfo(
        q.kCGWindowListOptionOnScreenOnly | q.kCGWindowListExcludeDesktopElements, q.kCGNullWindowID
    )
    return [
        int(window[q.kCGWindowLayer])
        for window in windows or ()
        if int(window[q.kCGWindowOwnerPID]) == pid
        and (float(window[q.kCGWindowBounds]["Width"]), float(window[q.kCGWindowBounds]["Height"])) == size
    ]


def _borderless(ui, pid, what, check=lambda size: True):
    """The shown panel: one AX window with no title bar buttons, over normal windows (a layer above 0)."""

    def panel():
        windows = _quick_windows(ui)
        assert len(windows) == 1, f"{what}: the quick terminal's windows are {windows}"
        ((size, buttons),) = windows
        assert buttons == [], f"{what}: the panel has the title bar buttons {buttons}"
        assert check(size), f"{what}: the panel's size is {size}"
        layers = _layers(ui, pid, size)
        assert layers and all(layer > 0 for layer in layers), f"{what}: the panel's window layers are {layers}"
        return size

    panel.__qualname__ = f"the borderless panel {what}"
    return wait_until(panel)


def _types_into_it(ctl, ui, marker):
    """A line typed key by key, as a keyboard sends it, runs in the quick terminal."""
    for char in f"echo {marker}":
        ui.press("space" if char == " " else char)
    ui.press("return")

    def ran():
        text = assert_ok(ctl.request("quick.text", all=True))["text"]
        assert re.search(rf"^{marker}\s*$", text, re.MULTILINE), text[-400:]
        return True

    ran.__qualname__ = f"{marker} printed in the quick terminal"
    wait_until(ran)


@pytest.mark.covers("ui:quick-terminal", "quick.text", "surface.zoom")
def test_the_quick_panel_has_no_title_bar_and_takes_the_keyboard(app, ctl, ui):
    pid = ctl.instance.pid
    ui.activate()
    try:
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        shown = _borderless(ui, pid, "shown")
        _types_into_it(ctl, ui, "quick-has-keys-1")

        assert_ok(ctl.request("surface.zoom", "quick", mode="show"))
        _borderless(ui, pid, "zoomed", lambda size: size[0] > shown[0] and size[1] > shown[1])
        _types_into_it(ctl, ui, "quick-has-keys-2")
        assert_ok(ctl.request("surface.zoom", "quick", mode="hide"))
        _borderless(ui, pid, "unzoomed", lambda size: size == shown)

        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is False)
        wait_until(lambda: _quick_windows(ui) == [])
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        _borderless(ui, pid, "shown again", lambda size: size == shown)
        _types_into_it(ctl, ui, "quick-has-keys-3")
    finally:
        ctl.request("surface.zoom", "quick", mode="hide")
        ctl.request("quick", mode="hide")
