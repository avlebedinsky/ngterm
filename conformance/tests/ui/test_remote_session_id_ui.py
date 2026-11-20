"""The origin ID after the user undoes a close of an attached row."""

import sys

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.since("0.42.1"),
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", reason="the origin session must be Live"),
]


@pytest.mark.covers("live.attach", "session.close", "events.read", "tree")
def test_close_undo_keeps_the_origin_id_in_the_row_and_events(remote_origin, remote_viewer, ui_for):
    origin = Ctl(remote_origin.instance)
    ui = ui_for(remote_viewer)
    sid = assert_ok(origin.request("session.new", name="undo-origin-id"))["id"]
    local = None
    try:
        origin.wait_mounted(sid, "left")
        origin.wait_response(
            origin.vocabulary.command("live.tree"), None,
            lambda response: any(row["id"] == sid for row in assert_ok(response)["remote"]["sessions"]),
        )
        local = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach"), sid, host=remote_origin.host)
        )["id"]
        cursor = remote_viewer.events_baseline()
        receipt = assert_ok(remote_viewer.request("session.close", local, undoable=True))
        assert receipt["undoable"] is True
        assert local not in session_ids(remote_viewer.tree())
        closed = remote_viewer.wait_event(cursor, "remote.closed", {"session": local})
        assert closed["payload"]["remoteSessionID"] == sid
        ui.press("cmd+z" if sys.platform == "darwin" else "ctrl+shift+z")
        restored = remote_viewer.wait_tree(lambda tree: local in session_ids(tree))
        assert find_session(restored, local)["remoteSessionID"] == sid
        opened = remote_viewer.wait_event(cursor.past(closed), "remote.opened", {"session": local})
        assert opened["payload"]["remoteSessionID"] == sid
    finally:
        if local is not None and local in session_ids(remote_viewer.tree()):
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))
