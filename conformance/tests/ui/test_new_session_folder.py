"""New Session from a workspace row with "Current session's directory": the folder comes from that workspace (#873).

The row's + button and its New Session menu item take the selection when it is inside the row's workspace, else the
workspace's most recent session, else its first; only an empty workspace takes the selection from elsewhere. A split
source gives its focused pane's folder (`AppStore+NewSessionPlacement.swift` `newSessionDirectorySource`,
`AppStoreOrganizationTests.directorySourceFollowsTheDestinationWorkspace`, `AppActionsTests` v0.42.0)."""

import os

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.since("0.42.0")


def _folder(ctl, name):
    path = ctl.instance.root / "folders" / name
    path.mkdir(parents=True)
    return str(path)


def _session(ctl, wid, cwd):
    sid = assert_ok(ctl.request("session.new", workspace=wid, cwd=cwd, noSelect=True))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _select(ctl, *sids):
    for sid in sids:
        assert_ok(ctl.request("session.select", sid))
        ctl.wait_tree(lambda tree, sid=sid: find_session(tree, sid).get("active") is True)


def _row(ui, name):
    return wait_until(
        lambda: next((row for row in ui.all("workspace-row") if ui.text(row) == name), None), timeout=ui.timeout
    )


def _in_row(ui, button, row):
    _, row_y, _, row_height = ui.frame(row)
    _, y, _, height = ui.frame(button)
    return row_y <= y + height / 2 <= row_y + row_height


def _new_session(ctl, ui, wid, name, entry):
    """New Session from the `name` row, by its + button or its menu item; the new session's cwd."""
    before = set(session_ids(ctl.tree(), wid))
    row = _row(ui, name)
    if entry == "button":
        # Drawn only under the pointer.
        ui.hover(row)
        ui.click(
            wait_until(
                lambda: next(
                    (b for b in ui.all("workspace-add-session") if b.role == "button" and _in_row(ui, b, row)), None
                ),
                timeout=ui.timeout,
            )
        )
    else:
        ui.mouse_click(row, button="right")
        ui.click(ui.menu_item("New Session"))
    tree = ctl.wait_tree(lambda tree: set(session_ids(tree, wid)) - before)
    (created,) = set(session_ids(tree, wid)) - before
    return os.path.realpath(find_session(tree, created)["cwd"])


@pytest.mark.parametrize("entry", ["button", "menu"])
@pytest.mark.covers("ui:workspace-add-session", "ui:workspace-row", "behavior:sidebar.row-context-menu")
def test_new_session_takes_its_folder_from_the_rows_workspace(launch, ui_for, entry):
    ctl = launch(settings={"newSessionDirectory": "currentSession"})
    ui = ui_for(ctl)
    (home,) = session_ids(ctl.tree())
    ctl.wait_mounted(home, "left")
    ids = {
        name: assert_ok(ctl.request("workspace.new", name=name))["id"]
        for name in ("work", "other", "unselected", "empty", "paned")
    }
    selected = _session(ctl, ids["work"], _folder(ctl, "selected"))
    first, recent, last = (_session(ctl, ids["other"], _folder(ctl, name)) for name in ("first", "recent", "last"))
    never = _folder(ctl, "never-selected")
    _session(ctl, ids["unselected"], never)
    _session(ctl, ids["unselected"], _folder(ctl, "also-never"))
    paned = _session(ctl, ids["paned"], _folder(ctl, "paned"))
    right = _folder(ctl, "right")
    assert_ok(ctl.request("session.split", paned, mode="on"))
    ctl.wait_mounted(paned, "left", "right")
    # The shell reports its folder by OSC 7, as a shell integration does.
    report = "printf '\\033]7;file://localhost%s\\033\\\\' \"$PWD\""
    assert_ok(ctl.request("session.type", paned, pane="right", text=f"cd '{right}' && {report}\n"))
    assert_ok(ctl.request("session.focus", paned, pane="right"))

    def right_focused(tree):
        node = find_session(tree, paned)
        reported = os.path.realpath(node.get("splitCwd", ""))
        return node.get("splitFocused") is True and reported == os.path.realpath(right)

    ctl.wait_tree(right_focused)
    folder = {sid: os.path.realpath(find_session(ctl.tree(), sid)["cwd"]) for sid in (selected, recent)}

    # The selection is elsewhere: the workspace's most recent session.
    _select(ctl, first, last, recent, selected)
    assert _new_session(ctl, ui, ids["other"], "other", entry) == folder[recent]
    # No session there was ever selected: its first.
    _select(ctl, selected)
    assert _new_session(ctl, ui, ids["unselected"], "unselected", entry) == os.path.realpath(never)
    # An empty workspace: the selection.
    _select(ctl, selected)
    assert _new_session(ctl, ui, ids["empty"], "empty", entry) == folder[selected]
    # A split source: its focused pane.
    _select(ctl, paned, selected)
    assert _new_session(ctl, ui, ids["paned"], "paned", entry) == os.path.realpath(right)
    # The selection inside the workspace.
    _select(ctl, selected)
    assert _new_session(ctl, ui, ids["work"], "work", entry) == folder[selected]
