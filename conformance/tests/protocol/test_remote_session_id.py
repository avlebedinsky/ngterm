"""The origin session id on attached rows and their events, through the private loopback sshd."""

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.since("0.42.0"),
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", reason="the origin session must be Live"),
]


@pytest.mark.parametrize("companion", [False, True], ids=["root", "companion"])
@pytest.mark.covers("live.attach", "events.read", "tree", "field:tree.session.remoteSessionID")
def test_attached_rows_and_events_report_the_origin_session_id(remote_origin, remote_viewer, companion):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="origin-id"))["id"]
    parent = sid
    if companion:
        sid = assert_ok(origin.request("session.companion.new", parent, name="companion-id", noSelect=True))["id"]
    local = None
    cursor = remote_viewer.events_baseline()
    try:
        origin.wait_mounted(sid, "left")
        local = assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"]
        node = find_session(remote_viewer.tree(), local)
        assert local != sid
        assert node["remoteSessionID"] == sid
        assert node["remoteHost"] == remote_origin.host
        opened = remote_viewer.wait_event(cursor, "remote.opened", {"session": local})
        assert opened["payload"]["remoteSessionID"] == sid
        assert opened["payload"]["host"] == remote_origin.host
        closed_id = local
        assert_ok(remote_viewer.request("session.close", local))
        local = None
        closed = remote_viewer.wait_event(cursor.past(opened), "remote.closed", {"session": closed_id})
        assert closed["payload"]["remoteSessionID"] == sid
        assert find_session(origin.tree(), sid)["id"] == sid
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", parent))
