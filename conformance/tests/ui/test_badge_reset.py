"""Real input clears the unseen badge under the selected mode (upstream #960, BadgeResetUITests)."""

import time

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl, launch_options
from agterm_conformance.wait import wait_until


@pytest.fixture
def badged(adapter, request):
    settings = {"notificationsEnabled": False}
    if request.param != "firstKey":
        settings["badgeReset"] = request.param
    instance = adapter.launch(settings=settings, **launch_options(request))
    try:
        yield Ctl(instance), request.param
    finally:
        adapter.stop(instance)


@pytest.mark.since("0.42.0")
@pytest.mark.covers("notify", "session.type", "session.focus", "session.split", "field:tree.session.unseen")
@pytest.mark.parametrize("badged", ["firstKey", "enter", "never"], indirect=True)
@pytest.mark.parametrize("pane", ["left", "right"])
def test_real_input_clears_the_badge_by_the_selected_mode(badged, pane, ui_for):
    ctl, mode = badged
    ui = ui_for(ctl)
    (sid, *_) = session_ids(ctl.tree())
    ctl.wait_mounted(sid, "left")
    if pane == "right":
        assert_ok(ctl.request("session.split", sid, mode="on"))
        ctl.wait_mounted(sid, "right")
        assert_ok(ctl.request("session.focus", sid, pane=pane))
    else:
        assert_ok(ctl.request("session.select", sid))
    wait_until(lambda: bool(find_session(ctl.tree(), sid).get("splitFocused")) == (pane == "right"))
    ui.type_text("true # badge-ready")
    ctl.wait_text(sid, "badge-ready", pane=pane)
    assert_ok(ctl.request("notify", sid, body="Badge reset probe"))
    wait_until(lambda: find_session(ctl.tree(), sid).get("unseen") == 1)

    def holds():
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            assert find_session(ctl.tree(), sid).get("unseen") == 1
            time.sleep(0.05)

    try:
        assert_ok(ctl.request("session.type", sid, pane=pane, text="true\n"))
        holds()
        ui.press("escape")
        if mode == "firstKey":
            wait_until(lambda: "unseen" not in find_session(ctl.tree(), sid))
            return
        holds()
        ui.type_text("true")
        holds()
        ui.press("shift+return")
        holds()
        ui.press("return")
        if mode == "enter":
            wait_until(lambda: "unseen" not in find_session(ctl.tree(), sid))
        else:
            holds()
    finally:
        ctl.request("session.seen", sid)
