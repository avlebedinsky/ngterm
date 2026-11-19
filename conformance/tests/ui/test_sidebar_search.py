"""The sidebar's session search (capability `sidebar-search`, a Rust-only extension; its ids are under `rust:` in
ui-ids.yaml): Search Sessions…'s chord gives the field the keyboard, typing narrows the rows to the sessions whose name
holds the text, Return opens the first match, a text matching nothing shows the empty hint with its Clear search link,
and Escape empties the field. The tree reads the field's text back as `sidebarSearch`. The test leaves the shared
instance's sessions and search as it found them."""

import pytest
from agterm_conformance.match import active_session_id, assert_ok
from agterm_conformance.wait import wait_until


def _new(ctl, workspace, name):
    sid = assert_ok(ctl.request("session.new", workspace=workspace))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.rename", sid, name=name))
    return sid


def _rows(ui):
    return [ui.text(element) for element in ui.all("session-row")]


def _search_of(tree):
    """The tree's `sidebarSearch`, None while it is omitted."""
    return tree["tree"].get("sidebarSearch")


def _searched(ctl, text):
    ctl.wait_tree(lambda tree: _search_of(tree) == text)


def _chord(ctl):
    """Search Sessions…'s chord as the keymap ships it on this OS."""
    listing = assert_ok(ctl.request("keymap.list"))["keymap"]
    (action,) = [action for action in listing["actions"] if action["action"] == "focus_sidebar_search"]
    return action["chord"]


@pytest.mark.requires("sidebar-search", reason="upstream's sidebar has no search")
@pytest.mark.covers(
    "ui:sidebar-search",
    "ui:sidebar-search-clear",
    "ui:sidebar-search-empty",
    "ui:sidebar-search-empty-clear",
    "ui:session-row",
)
def test_the_search_field_narrows_the_rows_and_return_opens_the_first_match(ctl, ui):
    wid = assert_ok(ctl.request("workspace.new", name="ui-search-ws"))["id"]
    try:
        _new(ctl, wid, "ui-search-alpha")
        beta = _new(ctl, wid, "ui-search-beta")
        other = _new(ctl, wid, "ui-plain")
        assert_ok(ctl.request("session.select", other))
        chord = _chord(ctl)

        ui.press(chord)
        ui.find("sidebar-search", role="text-field")
        ui.type_text("SEARCH-B")
        _searched(ctl, "SEARCH-B")
        wait_until(lambda: "ui-search-beta" in _rows(ui) and "ui-search-alpha" not in _rows(ui))
        assert "ui-plain" not in _rows(ui), "a session that does not match hides"
        assert active_session_id(ctl.tree()) == other, "typing never moves the selection"

        ui.press("return")
        ctl.wait_tree(lambda tree: active_session_id(tree) == beta)
        assert _search_of(ctl.tree()) == "SEARCH-B", "Return keeps the search"

        # the chord selects the field's text, so typing replaces it
        ui.press(chord)
        ui.type_text("zz-none")
        _searched(ctl, "zz-none")
        ui.find("sidebar-search-empty")
        ui.click(ui.find("sidebar-search-empty-clear"))
        _searched(ctl, None)
        ui.absent("sidebar-search-empty")

        ui.type_text("alpha")
        _searched(ctl, "alpha")
        ui.find("sidebar-search-clear")
        wait_until(lambda: "ui-search-alpha" in _rows(ui) and "ui-search-beta" not in _rows(ui))
        ui.press("escape")
        _searched(ctl, None)
        ui.absent("sidebar-search-clear")
        wait_until(lambda: {"ui-search-alpha", "ui-search-beta", "ui-plain"} <= set(_rows(ui)))
        assert active_session_id(ctl.tree()) == beta, "Escape keeps the selection"
    finally:
        if _search_of(ctl.tree()):
            ui.press(_chord(ctl), "escape")
        assert_ok(ctl.request("workspace.delete", wid))
