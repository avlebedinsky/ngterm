"""A question (`ngtermctl ask`) that a program in an attached session asks, answered where the user sees it: on the
viewer that presents the session. The origin is the staged `ngterm-headless` behind the harness's own sshd
(`remote_origin`), which draws nothing, so the viewer's dialog is the only place to answer it. The click goes back to
the origin, and the CLI there prints the answer."""

import json
import shlex

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]

ANSWER_FILE = "remote-ask-ui.json"


def _lead(sid, role):
    def leads(tree):
        (left,) = [surface for surface in find_session(tree, sid)["surfaces"] if surface["kind"] == "left"]
        assert left.get("lead") == role, left
        return True

    leads.__qualname__ = f"the left pane of {sid} reading {role}"
    return leads


@pytest.mark.parametrize("remote_origin", ["headless"], indirect=True)
@pytest.mark.parametrize("style", ["terminal", "gui"])
@pytest.mark.covers("ask.open", "ui:ask-dialog", "ui:ask-title", "ui:ask-button-{choice}")
def test_an_ask_from_a_headless_origin_is_answered_on_the_viewer(remote_origin, remote_viewer, ui_for, style):
    """The origin's own client has taken the lead, as after a takeover there: the ask still shows on the viewer, and
    its Yes reaches the CLI on the origin."""
    origin = Ctl(remote_origin.instance)
    home = remote_origin.instance.home
    sid = assert_ok(origin.request("session.new"))["id"]
    origin.wait_mounted(sid, "left")
    local = assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"]
    remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
    origin.wait_tree(lambda tree: find_session(tree, sid).get("presenters", {}).get("presenter") is True)
    origin.wait_tree(_lead(sid, "follower"))
    assert_ok(origin.request("session.lead", sid))
    origin.wait_tree(_lead(sid, "leader"))

    answer = home / ANSWER_FILE
    answer.unlink(missing_ok=True)
    title = f"Deploy from the {style} ask?"
    command = (
        f"{remote_origin.adapter.ctl_name} ask {shlex.quote(title)} --button yes=Yes --button no=No --style {style} "
        f'--target "${origin.instance.variable("SESSION_ID")}" > {shlex.quote(ANSWER_FILE)} 2>&1\n'
    )
    assert_ok(origin.request("session.type", sid, text=command))
    ask = find_session(origin.wait_tree(lambda tree: find_session(tree, sid).get("ask")), sid)["ask"]
    assert ask.get("remote") is True, ask

    ui = ui_for(remote_viewer)
    ui.find("ask-dialog")
    assert ui.text("ask-title") == title
    ui.click(ui.find(ui.ui_ids.expand("ask-button-{choice}", choice="yes"), name="Yes"))

    def written():
        text = answer.read_text() if answer.exists() else ""
        assert text.strip(), f"{answer} is empty"
        return text

    assert json.loads(wait_until(written)) == {"result": "answered", "id": "yes", "label": "Yes", "index": 0}
    ui.absent("ask-dialog")
    remote_viewer.wait_tree(lambda tree: "ask" not in find_session(tree, local))
    origin.wait_tree(lambda tree: "ask" not in find_session(tree, sid))

    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))
