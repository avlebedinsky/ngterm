"""Custom commands of `keymap.conf` fired by their chords: the chord goes in through the OS's keyboard input, the key
monitor runs the command's line, and the tree reads back what the line did. Each test launches its own instance, so
it knows the whole sidebar; the file is the instance's own `<state>/config/keymap.conf`. Every line calls the build's
own CLI with an explicit `--socket` and writes only into the instance root, in the build's command-line dialect
(`agterm_conformance.command_line`)."""

import pytest
from agterm_conformance.command_line import CommandLines
from agterm_conformance.match import active_session_id, assert_ok, session_ids
from agterm_conformance.wait import wait_until


def _stderr_into(lines, command, path):
    """`command`, its error output into `path` through a temp file renamed into place: the file appears once the
    command has run, empty when it printed no error."""
    tmp, target = lines.quote(f"{path}.tmp"), lines.quote(path)
    move = f"move /y {tmp} {target} >nul" if lines.cmd else f"mv {tmp} {target}"
    return lines.then(f"{command} 2> {tmp}", move)


def _written(path):
    """The text of `path` once it exists, empty text included."""

    def written():
        assert path.exists(), f"{path.name} not written yet"
        return True

    written.__qualname__ = f"{path.name} written"
    wait_until(written)
    return path.read_text()


@pytest.mark.since("0.42.0")
@pytest.mark.covers("session.go", "keymap.reload", "workspace.collapse")
def test_a_chord_bound_to_session_go_to_n_selects_the_nth_session(adapter, launch, ui_for):
    # upstream 487d643b: `session go --to N` makes a chord per sidebar place nine keymap command lines; Control-1 and
    # Control-2 stay the pane shortcuts, so the chords here start at Control-3.
    lines = CommandLines.of(adapter)
    ctl = launch()
    ui = ui_for(ctl)
    wid = assert_ok(ctl.request("workspace.new"))["id"]
    for _ in range(3):
        ctl.wait_mounted(assert_ok(ctl.request("session.new", workspace=wid))["id"], "left")
    ids = session_ids(ctl.tree())
    past = len(ids) + 1
    assert 3 < len(ids) < 9, ids
    assert_ok(ctl.request("workspace.collapse", wid))
    assert_ok(ctl.request("session.select", ids[0]))

    go = f"{lines.quote(adapter.ctl_path)} session go --socket {lines.quote(ctl.instance.socket)} --to"
    markers = {n: ctl.instance.root / f"go-{n}" for n in (3, len(ids), past)}
    keymap = ctl.instance.state_dir / "config" / "keymap.conf"
    keymap.write_text(
        "".join(f'command "go {n}" ctrl+{n} {_stderr_into(lines, f"{go} {n}", path)}\n' for n, path in markers.items())
    )
    assert assert_ok(ctl.request("keymap.reload")) == {"count": 0}

    for n in (3, len(ids)):
        ui.press(f"ctrl+{n}")
        assert _written(markers[n]) == "", n
        ctl.wait_tree(lambda tree, n=n: active_session_id(tree) == ids[n - 1])

    ui.press(f"ctrl+{past}")
    assert _written(markers[past]).strip() == f"error: no session at position {past}: {len(ids)} navigable"
    assert active_session_id(ctl.tree()) == ids[-1], "a place past the end keeps the selection"
