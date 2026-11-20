"""The origin session id on attached rows and their events, through the private loopback sshd."""

import re

import pytest
from agterm_conformance import cli_format
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Cli, Ctl

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
        origin.wait_response(
            origin.vocabulary.command("live.tree"), None,
            lambda response: any(row["id"] == sid for row in assert_ok(response)["remote"]["sessions"]),
        )
        local = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach"), sid, host=remote_origin.host)
        )["id"]
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


@pytest.mark.since("0.42.1")
@pytest.mark.covers("live.attach", "events.read", "cli:events", "tree", "session.companion.page")
def test_page_companion_rows_and_human_events_report_the_origin_id(remote_origin, remote_viewer, adapter):
    origin = Ctl(remote_origin.instance)
    parent = assert_ok(origin.request("session.new", name="page-parent"))["id"]
    local = None
    try:
        origin.wait_mounted(parent, "left")
        origin.wait_response(
            origin.vocabulary.command("live.tree"), None,
            lambda response: any(row["id"] == parent for row in assert_ok(response)["remote"]["sessions"]),
        )
        local = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach"), parent, host=remote_origin.host)
        )["id"]
        path = origin.instance.root / "origin-id.html"
        path.write_text("<!doctype html><title>Origin ID</title><p>Origin page companion</p>")
        page = assert_ok(origin.request("session.companion.page", parent, html=str(path)))["id"]
        cursor = remote_viewer.events_baseline()
        attached = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach"), page, host=remote_origin.host)
        )["id"]
        node = find_session(remote_viewer.tree(), attached)
        assert node["remoteSessionID"] == page
        assert node["parent"] == local
        assert node["content"] == "page"
        opened = remote_viewer.wait_event(cursor, "remote.opened", {"session": attached})
        assert opened["payload"]["remoteSessionID"] == page
        assert_ok(remote_viewer.request("session.close", attached))
        closed = remote_viewer.wait_event(cursor.past(opened), "remote.closed", {"session": attached})
        assert closed["payload"]["remoteSessionID"] == page
        cli = Cli(adapter.ctl_path, remote_viewer.instance, adapter.ctl_name)
        with cli.spawn(
            "events", "--run", cursor.run, "--after", str(cursor.after),
            "--kind", "remote.opened,remote.closed",
        ) as stream:
            lines = [stream.line(), stream.line()]
        assert lines == [cli_format.event(event) for event in (opened, closed)]
        for line, kind in zip(lines, ("remote.opened", "remote.closed")):
            assert re.fullmatch(
                rf"\d{{2}}:\d{{2}}:\d{{2}} {re.escape(kind)} .+ "
                rf"host={re.escape(remote_origin.host)} remote-session-id={re.escape(page)}", line
            )
        assert find_session(origin.tree(), page)["id"] == page
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", parent))
