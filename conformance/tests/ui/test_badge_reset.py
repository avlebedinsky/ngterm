"""Real input clears the unseen badge under the selected mode (upstream #960, BadgeResetUITests)."""

import json
import time

import pytest
from agterm_conformance.drivers.base import ElementNotFound
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl, launch_options
from agterm_conformance.wait import DEFAULT_TIMEOUT, wait_until


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


PICKER = "settings-badge-reset"


def _open_notifications(ui):
    try:
        found = ui.tab("Notifications", timeout=1)
    except ElementNotFound:
        # Linux and Windows have no Settings chord: the app menu's item opens it (tests/ui/test_settings.py).
        if ui.name in ("atspi", "uia"):
            ui.menu("ngterm", "Settings…")
        else:
            ui.press("cmd+,")
        found = ui.tab("Notifications")
    ui.click(found)


def _choose(ui, item):
    """Pick `item` from the Badge reset picker's menu, pressing the picker again while the open menu lacks it."""

    def opened():
        if not ui.all(PICKER):
            _open_notifications(ui)
        ui.click(ui.find(PICKER, role="popup"))
        try:
            return ui.menu_item(item, timeout=DEFAULT_TIMEOUT)
        except ElementNotFound as error:
            ui.press("escape")
            raise AssertionError(f"the open menu has no {item!r}: {error}") from None

    ui.click(wait_until(opened, timeout=3 * DEFAULT_TIMEOUT))


def _stored(ctl):
    return json.loads((ctl.instance.state_dir / "settings.json").read_text(encoding="utf-8")).get("badgeReset")


def _badge_main_pane(ctl, ui, marker):
    (sid, *_) = session_ids(ctl.tree())
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.select", sid))
    ui.type_text(f"true # {marker}")
    ctl.wait_text(sid, marker)
    assert_ok(ctl.request("notify", sid, body="Badge reset probe"))
    wait_until(lambda: find_session(ctl.tree(), sid).get("unseen") == 1)
    return sid


@pytest.mark.since("0.42.0")
@pytest.mark.covers("ui:settings-badge-reset", "notify", "field:tree.session.unseen")
def test_badge_reset_reads_an_unknown_value_as_first_key_and_survives_a_relaunch(adapter, request, ui_for):
    def value(text):
        return {"value": text} if adapter.has("ui-string-values") else {}

    instance = adapter.launch(
        settings={"notificationsEnabled": False, "badgeReset": "sometimes"}, **launch_options(request)
    )
    relaunching = False
    try:
        ctl = Ctl(instance)
        ui = ui_for(ctl)
        sid = _badge_main_pane(ctl, ui, "unknown-mode-ready")
        ui.press("escape")
        wait_until(lambda: "unseen" not in find_session(ctl.tree(), sid))
        _open_notifications(ui)
        ui.find(PICKER, role="popup", **value("On first key"))
        _choose(ui, "On Enter")
        wait_until(lambda: _stored(ctl) == "enter")

        relaunching = True
        instance = adapter.relaunch(instance)
        relaunching = False
        ctl = Ctl(instance)
        ui = ui_for(ctl)
        assert _stored(ctl) == "enter"
        sid = _badge_main_pane(ctl, ui, "relaunched-ready")
        try:
            ui.press("escape")
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                assert find_session(ctl.tree(), sid).get("unseen") == 1
                time.sleep(0.05)
            ui.press("return")
            wait_until(lambda: "unseen" not in find_session(ctl.tree(), sid))
        finally:
            ctl.request("session.seen", sid)
        _open_notifications(ui)
        ui.find(PICKER, role="popup", **value("On Enter"))
    finally:
        # A failed relaunch has stopped what it started and keeps the root for its log.
        if not relaunching:
            adapter.stop(instance)
