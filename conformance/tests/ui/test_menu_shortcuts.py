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
# AppKit's key equivalent characters (NSCarriageReturnCharacter, the NSUpArrowFunctionKey family). Delete is
# NSBackspaceCharacter, SwiftUI's `.delete`: AppKit shows it as ⌫ and NSDeleteCharacter (0x7F) as Forward Delete ⌦.
KEYS = {
    "return": ("\r",),
    "tab": ("\t",),
    "space": (" ",),
    "delete": ("\x08",),
    "up": ("\uf700",),
    "down": ("\uf701",),
    # NSF1FunctionKey onward
    **{f"f{n}": (chr(0xF703 + n),) for n in range(1, 21)},
}
# kAXMenuItemModifier*: Command is held unless the no-command bit is set
SHIFT, OPTION, CONTROL, NO_COMMAND = 1, 2, 4, 8
MODIFIERS = {"shift": SHIFT, "opt": OPTION, "ctrl": CONTROL}


def _chords(ctl):
    listing = assert_ok(ctl.request("keymap.list"))["keymap"]
    return {action["action"]: action.get("chord") for action in listing["actions"]}


def _expected(chord):
    """The AX characters that can show a chord such as `cmd+shift+return`, and its modifier mask."""
    *mods, key = chord.split("+")
    mask = 0 if "cmd" in mods else NO_COMMAND
    for mod in mods:
        mask |= MODIFIERS.get(mod, 0)
    # AX reads a letter's key equivalent upper case, Shift or not
    return KEYS.get(key, (key, key.upper())), mask


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
            char, mask = _shown(ui, path)
            assert char in want[0] and mask == want[1], (
                f"{' ▸ '.join(path)} shows {(char, mask)!r}, its chord {chord} is one of {want[0]!r} with {want[1]!r}"
            )
            return True

        shown.__qualname__ = f"{' ▸ '.join(path)} shows {chord}"
        wait_until(shown)


def _app_menu_item(ui, prefix):
    """The app menu's item whose title starts with `prefix` (its title names the app)."""
    for top in ui.attribute(ui.attribute(ui.app, "AXMenuBar"), "AXChildren") or ():
        for menu in ui.attribute(top, "AXChildren") or ():
            for item in ui.attribute(menu, "AXChildren") or ():
                if (ui.attribute(item, "AXTitle") or "").startswith(prefix):
                    return item
    raise AssertionError(f"no menu item {prefix}…")


def test_quit_and_settings_show_their_command_chords(ui):
    for prefix, key in (("Quit", "q"), ("Settings", ",")):

        def shown(prefix=prefix, key=key):
            item = _app_menu_item(ui, prefix)
            shown = ui.attribute(item, "AXMenuItemCmdChar"), ui.attribute(item, "AXMenuItemCmdModifiers")
            assert shown in ((key, 0), (key.upper(), 0)), f"{prefix}… shows {shown!r}, not Command-{key}"
            return True

        shown.__qualname__ = f"{prefix}… shows Command-{key}"
        wait_until(shown)


# a function key on a keyed item and on a keyless one, and Delete, which shows ⌫ (upstream's View ▸ Hide Sidebar,
# Navigate ▸ Last Session, File ▸ Clear Status)
BOUND_KEY_ITEMS = (
    ("toggle_sidebar", "f5", ("View", "Hide Sidebar")),
    ("last_session", "f12", ("Navigate", "Last Session")),
    ("clear_status", "ctrl+opt+delete", ("File", "Clear Status")),
)


def _wait_shown(ui, path, chord):
    def shown():
        char, mask = _shown(ui, path)
        if chord is None:
            assert char is None, f"{' ▸ '.join(path)} still shows {(char, mask)!r}"
        else:
            want = _expected(chord)
            assert char in want[0] and mask == want[1], (
                f"{' ▸ '.join(path)} shows {(char, mask)!r}, its chord {chord} is one of {want[0]!r} with {want[1]!r}"
            )
        return True

    shown.__qualname__ = f"{' ▸ '.join(path)} shows {chord}"
    wait_until(shown)


@pytest.mark.covers("keymap.reload", "keymap.list")
def test_a_named_key_binding_shows_in_the_menu_until_it_is_removed(app, ctl, ui):
    defaults = _chords(ctl)
    keymap = ctl.instance.state_dir / "config" / "keymap.conf"
    keymap.parent.mkdir(parents=True, exist_ok=True)
    keymap.write_text("".join(f"map {key} {action}\n" for action, key, _ in BOUND_KEY_ITEMS))
    assert assert_ok(ctl.request("keymap.reload")) == {"count": 0}
    chords = _chords(ctl)
    for action, key, path in BOUND_KEY_ITEMS:
        assert chords[action] == key, (action, chords[action])
        _wait_shown(ui, path, key)

    keymap.write_text("")
    assert assert_ok(ctl.request("keymap.reload")) == {"count": 0}
    assert _chords(ctl) == defaults
    for action, _, path in BOUND_KEY_ITEMS:
        _wait_shown(ui, path, defaults[action])
