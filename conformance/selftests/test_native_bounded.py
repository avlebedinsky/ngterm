"""scripts/lib/native.sh's `bounded`, the time limit the iOS scripts put on each simulator wait and build-ghostty-vt.sh
on each Zig fetch: a command that ends in time keeps its status and output and returns at once, one that does not is
stopped and gives 124, and the caller's command substitution gets its result when the limit runs out, not when the
stopped command would have ended. The callers run under `set -euo pipefail`, so every case does too."""

import shutil
import subprocess
import time
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]


def bounded(snippet: str) -> tuple[int, str, str, float]:
    """Run snippet in a `set -euo pipefail` bash with native.sh sourced; its status, stdout, stderr and wall time."""
    start = time.monotonic()
    done = subprocess.run(
        ["bash", "-c", f'set -euo pipefail\nsource "{REPO}/scripts/lib/native.sh"\n{snippet}'],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    return done.returncode, done.stdout, done.stderr, time.monotonic() - start


def test_a_command_that_ends_in_time_keeps_its_status_and_output():
    status, output, _, _ = bounded("status=0; bounded 30 sh -c 'echo done; exit 3' || status=$?; echo \"status $status\"")
    assert (status, output) == (0, "done\nstatus 3\n")


def test_a_command_that_ends_at_once_returns_at_once():
    status, output, _, elapsed = bounded('bounded 30 true; out="$(bounded 30 echo hi)"; echo "$out"')
    assert (status, output) == (0, "hi\n")
    assert elapsed < 0.9


def test_a_command_past_its_limit_is_stopped_with_status_124():
    status, output, stderr, elapsed = bounded(
        'status=0; out="$(bounded 1 sleep 30)" || status=$?; echo "status $status output [$out]"'
    )
    assert (status, output) == (0, "status 124 output []\n")
    assert "sleep 30 still running after 1s" in stderr
    assert elapsed < 10


def test_a_bare_call_past_its_limit_stops_a_set_e_caller_with_status_124():
    status, output, _, elapsed = bounded("bounded 1 sleep 30; echo after")
    assert (status, output) == (124, "")
    assert elapsed < 10


@pytest.mark.skipif(shutil.which("perl") is None, reason="needs perl for a command that ignores TERM")
def test_a_command_that_ignores_term_is_killed():
    status, output, _, elapsed = bounded(
        """status=0
bounded 1 perl -e '$| = 1; $SIG{TERM} = "IGNORE"; print "up\\n"; sleep 50' || status=$?
echo "status $status" """
    )
    assert (status, output) == (0, "up\nstatus 124\n")
    assert elapsed < 20
