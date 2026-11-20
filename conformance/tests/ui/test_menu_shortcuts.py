"""The macOS menu bar shows the chord of a menu item on a named key: macOS AX reads the item's key equivalent as
AppKit's character for the key (a carriage return for Return, the function-key characters for the arrows) and its
modifiers. The chord comes from the build's own keymap (`keymap.list`), so the test holds for any shipped table.
Upstream's `agtermApp.toShortcut` maps each named key to its SwiftUI `KeyEquivalent`."""

import sys

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="menu key equivalents as macOS AX reads them")

# the menu items of named-key chords: upstream's View ▸ Toggle Terminal Zoom and its Navigate menu's session steps
ITEMS = (
    ("toggle_terminal_zoom", ("View", "Toggle Terminal Zoom")),
    ("previous_session", ("Navigate", "Previous Session")),
    ("next_session", ("Navigate", "Next Session")),
)
# AppKit's key equivalent characters (NSCarriageReturnCharacter, the NSUpArrowFunctionKey family)
KEYS = {"return": "\r", "tab": "\t", "space": " ", "delete": "\x7f", "up": "", "down": ""}
# kAXMenuItemModifier*: Command is held unless the no-command bit is set
SHIFT, OPTION, CONTROL, NO_COMMAND = 1, 2, 4, 8
MODIFIERS = {"shift": SHIFT, "opt": OPTION, "ctrl": CONTROL}


def _chords(ctl):
    listing = assert_ok(ctl.request("keymap.list"))["keymap"]
    return {action["action"]: action.get("chord") for action in listing["actions"]}


def _expected(chord):
    """The AX character and modifier mask of a chord such as `cmd+shift+return`."""
    *mods, key = chord.split("+")
    mask = 0 if "cmd" in mods else NO_COMMAND
    for mod in mods:
        mask |= MODIFIERS.get(mod, 0)
    return KEYS[key], mask


def _menu_item(ui, path):
    """The AX menu item at `path` down the menu bar; each level's items sit under its one AXMenu child."""
    level = ui.attribute(ui.app, "AXMenuBar")
    for depth, title in enumerate(path):
        children = ui.attribute(level, "AXChildren") or ()
        item = next((child for child in children if ui.attribute(child, "AXTitle") == title), None)
        assert item is not None, f"no menu item {' ▸ '.join(path[: depth + 1])}"
        if depth == len(path) - 1:
            return item
        level = next(iter(ui.attribute(item, "AXChildren") or ()), None)
    raise AssertionError(f"empty menu path {path}")


def _shown(ui, path):
    item = _menu_item(ui, path)
    return ui.attribute(item, "AXMenuItemCmdChar"), ui.attribute(item, "AXMenuItemCmdModifiers")


@pytest.mark.covers("keymap.list")
def test_menu_items_on_named_keys_show_their_chord(ctl, ui):
    chords = _chords(ctl)
    for action, path in ITEMS:
        chord = chords.get(action)
        assert chord, f"{action} has no chord in the shipped keymap"
        want = _expected(chord)

        def shown(path=path, want=want, chord=chord):
            got = _shown(ui, path)
            assert got == want, f"{' ▸ '.join(path)} shows {got!r}, its chord {chord} is {want!r}"
            return True

        shown.__qualname__ = f"{' ▸ '.join(path)} shows {chord}"
        wait_until(shown)
