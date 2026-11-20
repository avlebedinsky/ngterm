"""The quick terminal's panel on macOS, as the user meets it. The in-app chord (ctrl+`, through the OS's keyboard
input) shows it without a title bar: its AX window has none of the title bar's buttons, also when it is zoomed. It
floats over normal windows, and it takes the keyboard: the keys typed next go to its terminal. Upstream's
`QuickTerminalPanel` is `[.borderless, .nonactivatingPanel]` at the floating level and can become key. Its frame is
upstream's `targetFrame`: the panel size of its screen's visible frame (the menu bar and Dock left out), centered in
it, and the whole visible frame while zoomed."""

import re
import sys

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="the panel's title bar buttons as macOS AX has them")

BUTTONS = ("AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton")
MAX_DEPTH = 40
# `QuickTerminalMetrics`' built-in size: a share of the visible frame, at most this size
DEFAULT_SHARE = 0.9
DEFAULT_MAX_SIZE = (1100.0, 700.0)
# both builds round the panel's size and its origin to whole points: an edge is up to one point off the unrounded frame
SLACK = 1.0


def _holds_quick(ui, handle, depth=0):
    if str(ui.attribute(handle, "AXIdentifier") or "") == "quick-terminal":
        return True
    return depth < MAX_DEPTH and any(
        _holds_quick(ui, child, depth + 1) for child in ui.attribute(handle, "AXChildren") or ()
    )


def _frame(ui, window):
    """The AX window's frame: x, y from the primary screen's top left, width, height."""
    ok, origin = ui.ax.AXValueGetValue(ui.attribute(window, "AXPosition"), ui.ax.kAXValueCGPointType, None)
    assert ok, "the panel's AX window has no position"
    ok, size = ui.ax.AXValueGetValue(ui.attribute(window, "AXSize"), ui.ax.kAXValueCGSizeType, None)
    assert ok, "the panel's AX window has no size"
    return float(origin.x), float(origin.y), float(size.width), float(size.height)


def _flipped(rect, primary_height):
    """An AppKit rect (bottom-left origin) as x, y from the primary screen's top left, width, height."""
    return (
        float(rect.origin.x),
        float(primary_height - rect.origin.y - rect.size.height),
        float(rect.size.width),
        float(rect.size.height),
    )


def _visible_frame(frame):
    """The visible frame of the screen that holds the center of `frame`, as `_frame` gives it."""
    import AppKit

    screens = AppKit.NSScreen.screens()
    primary_height = screens[0].frame().size.height
    x, y = frame[0] + frame[2] / 2, frame[1] + frame[3] / 2
    for screen in screens:
        left, top, width, height = _flipped(screen.frame(), primary_height)
        if left <= x < left + width and top <= y < top + height:
            return _flipped(screen.visibleFrame(), primary_height)
    raise AssertionError(f"no screen holds the panel's frame {frame}")


def _target_frame(visible):
    """Upstream's `targetFrame` unzoomed: the built-in panel size of the visible frame, centered in it."""
    left, top, width, height = visible
    panel = [min(extent * DEFAULT_SHARE, cap) for extent, cap in zip((width, height), DEFAULT_MAX_SIZE)]
    return left + (width - panel[0]) / 2, top + (height - panel[1]) / 2, *panel


def _framed(zoomed):
    """A check that the panel's frame is the visible frame (zoomed), else the target frame in it."""

    def check(frame):
        visible = _visible_frame(frame)
        want = visible if zoomed else _target_frame(visible)
        return all(abs(got - expected) <= SLACK for got, expected in zip(frame, want))

    return check


def _quick_windows(ui):
    """The frame and the title bar buttons of each AX window that holds the quick terminal."""
    return [
        (_frame(ui, window), [name for name in BUTTONS if ui.attribute(window, name) is not None])
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


def _borderless(ui, pid, what, check):
    """The shown panel: one AX window with no title bar buttons, over normal windows (a layer above 0), whose frame
    passes `check`."""

    def panel():
        windows = _quick_windows(ui)
        assert len(windows) == 1, f"{what}: the quick terminal's windows are {windows}"
        ((frame, buttons),) = windows
        assert buttons == [], f"{what}: the panel has the title bar buttons {buttons}"
        assert check(frame), f"{what}: the panel's frame is {frame} in the visible frame {_visible_frame(frame)}"
        layers = _layers(ui, pid, frame[2:])
        assert layers and all(layer > 0 for layer in layers), f"{what}: the panel's window layers are {layers}"
        return frame

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
        _borderless(ui, pid, "shown", _framed(zoomed=False))
        _types_into_it(ctl, ui, "quick-has-keys-1")

        assert_ok(ctl.request("surface.zoom", "quick", mode="show"))
        _borderless(ui, pid, "zoomed", _framed(zoomed=True))
        _types_into_it(ctl, ui, "quick-has-keys-2")
        assert_ok(ctl.request("surface.zoom", "quick", mode="hide"))
        _borderless(ui, pid, "unzoomed", _framed(zoomed=False))

        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is False)
        wait_until(lambda: _quick_windows(ui) == [])
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        _borderless(ui, pid, "shown again", _framed(zoomed=False))
        _types_into_it(ctl, ui, "quick-has-keys-3")
    finally:
        ctl.request("surface.zoom", "quick", mode="hide")
        ctl.request("quick", mode="hide")
