"""File ▸ Attach Remote on a host that runs ngterm, driven as a user does: the session picker of the one host in
`remotes.conf` puts Attach All and New Session above the sessions, Attach All counts the sessions not attached here,
Enter on the untouched picker takes Attach All, and a typed name takes its session. The origin is the staged
`ngterm-headless` behind the harness's own sshd (`remote_origin`); each test opens its own viewer and ends the origin
sessions it made."""

import contextlib
import time

import pytest
from agterm_conformance.match import assert_ok, sessions
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's sessions must be Live in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]

ATTACH_ALL = "attach-all"
NEW_SESSION = "new-session"
# a failed Attach All's panel stays 5 s (attach.rs ERROR_SECONDS), far longer than a poll
SETTLE_SECONDS = 1.0


@contextlib.contextmanager
def _origin_sessions(remote_origin, *names):
    """The origin's only sessions: named `names`, mounted, by name; ended on the way out. The sessions it had before
    (`ngterm-headless` starts with one) end first."""
    origin = Ctl(remote_origin.instance)
    for session in sessions(origin.tree()):
        assert_ok(origin.request("session.close", session["id"]))
    origin.wait_tree(lambda tree: not sessions(tree))
    made = {}
    try:
        for name in names:
            made[name] = assert_ok(origin.request("session.new", name=name))["id"]
            origin.wait_mounted(made[name], "left")
        yield made
    finally:
        for sid in made.values():
            origin.request("session.close", sid)


def _viewer(launch, remote_origin):
    viewer = launch(path_prefix=remote_origin.viewer_path)
    (viewer.instance.state_dir / "config").mkdir(exist_ok=True)
    (viewer.instance.state_dir / "config" / "remotes.conf").write_text(f"{remote_origin.host} Origin\n")
    return viewer


def _remote_rows(ctl):
    return [s for s in sessions(ctl.tree()) if s.get("remoteHost") is not None]


def _item(ui, item):
    return ui.ui_ids.expand("palette-item-{item}", item=item)


def _open_picker(ui, *wanted):
    """File ▸ Attach Remote… until the rows `wanted` (palette item ids) show."""
    ui.menu("File", "Attach Remote…")
    ui.find("pick-palette")
    for item in wanted:
        ui.find(_item(ui, item))


def _rows_top_down(ui, items):
    """`items` (palette item ids) sorted by where they sit, top first."""
    return sorted(items, key=lambda item: ui.frame(_item(ui, item))[1])


def _subtitles(ui, count):
    def enough():
        texts = [ui.text(element) for element in ui.all("palette-subtitle")]
        return texts if len(texts) >= count else None

    return wait_until(enough, timeout=ui.timeout)


def _panels_until_quiet(viewer):
    """The messages of every panel the viewer's sessions show until none shows, past SETTLE_SECONDS."""
    seen = []
    started = time.monotonic()

    def quiet():
        shown = [s["hud"]["message"] for s in sessions(viewer.tree()) if s.get("hud")]
        seen.extend(shown)
        return not shown and time.monotonic() - started > SETTLE_SECONDS

    wait_until(quiet)
    return seen


def _attached_names(viewer):
    return sorted(s["name"] for s in _remote_rows(viewer))


def _wait_attached(viewer, names):
    """Until the viewer's remote rows are named `names` (sorted)."""

    def attached(tree):
        shown = sorted(s["name"] for s in sessions(tree) if s.get("remoteHost") is not None)
        assert shown == names, shown
        return True

    viewer.wait_tree(attached, timeout=30)


@pytest.mark.covers("ui:pick-palette", "ui:palette-item-{item}")
def test_a_host_with_no_sessions_offers_new_session_first_and_no_attach_all(launch, remote_origin, ui_for):
    with _origin_sessions(remote_origin):
        viewer = _viewer(launch, remote_origin)
        ui = ui_for(viewer)
        _open_picker(ui, NEW_SESSION)
        assert ui.text(ui.find(_item(ui, NEW_SESSION))) == "New Session"
        assert not ui.all(_item(ui, ATTACH_ALL)), ui.tree_dump()
        assert _subtitles(ui, 1)[0] == f"create a session on {remote_origin.host} and attach it"
        ui.press("escape")
        ui.absent("pick-palette")
        assert _remote_rows(viewer) == []


@pytest.mark.covers("ui:pick-palette", "ui:palette-item-{item}", "ui:palette-subtitle")
def test_attach_all_and_new_session_come_first_and_enter_attaches_every_session(launch, remote_origin, ui_for):
    """Enter on the untouched picker takes its first row, Attach All; opened again, the row says all are attached, and
    choosing it attaches nothing and shows no error."""
    with _origin_sessions(remote_origin, "alpha", "beta", "gamma") as made:
        viewer = _viewer(launch, remote_origin)
        ui = ui_for(viewer)
        _open_picker(ui, ATTACH_ALL, NEW_SESSION, *made.values())
        order = [ATTACH_ALL, NEW_SESSION, *made.values()]
        assert _rows_top_down(ui, order)[:2] == [ATTACH_ALL, NEW_SESSION]
        assert ui.text(ui.find(_item(ui, ATTACH_ALL))) == "Attach All"
        subtitles = _subtitles(ui, 2)
        new_session = f"create a session on {remote_origin.host} and attach it"
        assert subtitles[:2] == ["3 sessions not attached here", new_session], subtitles

        ui.press("return")
        ui.absent("pick-palette")
        _wait_attached(viewer, ["alpha", "beta", "gamma"])

        _open_picker(ui, ATTACH_ALL)
        assert _subtitles(ui, 1)[0] == "all sessions are attached here"
        ui.press("return")
        ui.absent("pick-palette")
        assert set(_panels_until_quiet(viewer)) <= {"Attach Remote: attaching all sessions…"}
        assert _attached_names(viewer) == ["alpha", "beta", "gamma"]


@pytest.mark.covers("ui:pick-palette", "ui:palette-item-{item}", "ui:palette-subtitle")
def test_attach_all_offers_the_one_session_not_attached_here(launch, remote_origin, ui_for):
    with _origin_sessions(remote_origin, "kept", "left") as made:
        viewer = _viewer(launch, remote_origin)
        assert_ok(viewer.request("live.attach", made["kept"], host=remote_origin.host))
        _wait_attached(viewer, ["kept"])
        ui = ui_for(viewer)
        _open_picker(ui, ATTACH_ALL, NEW_SESSION)
        assert _subtitles(ui, 1)[0] == "1 session not attached here"
        ui.press("return")
        ui.absent("pick-palette")
        _wait_attached(viewer, ["kept", "left"])


@pytest.mark.covers("ui:pick-palette", "ui:palette-item-{item}")
def test_a_typed_session_name_and_enter_attach_that_session_alone(launch, remote_origin, ui_for):
    with _origin_sessions(remote_origin, "build", "deploy") as made:
        viewer = _viewer(launch, remote_origin)
        ui = ui_for(viewer)
        _open_picker(ui, ATTACH_ALL, made["deploy"])
        ui.type_text("deploy")
        wait_until(lambda: not ui.all(_item(ui, ATTACH_ALL)), timeout=ui.timeout)
        ui.press("return")
        ui.absent("pick-palette")
        _wait_attached(viewer, ["deploy"])
