"""scripts/lib/native.sh's `bounded`, the time limit the iOS scripts put on each simulator wait: a command that ends in
time keeps its status and output, one that does not is stopped and gives 124, and the caller's command substitution
gets its result when the limit runs out, not when the stopped command would have ended."""

import shutil
import subprocess
import time
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]


def bounded(snippet: str) -> tuple[int, str, float]:
    """Run snippet in bash with native.sh sourced; its status, its output and its wall time in seconds."""
    start = time.monotonic()
    done = subprocess.run(
        ["bash", "-c", f'source "{REPO}/scripts/lib/native.sh" && {snippet}'],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    return done.returncode, done.stdout, time.monotonic() - start


def test_a_command_that_ends_in_time_keeps_its_status_and_output():
    status, output, _ = bounded("bounded 30 sh -c 'echo done; exit 3'")
    assert (status, output) == (3, "done\n")


def test_a_command_past_its_limit_is_stopped_with_status_124():
    status, output, elapsed = bounded('out="$(bounded 1 sleep 30)"; echo "status $? output [$out]"')
    assert (status, output) == (0, "status 124 output []\n")
    assert elapsed < 10


@pytest.mark.skipif(shutil.which("perl") is None, reason="needs perl for a command that ignores TERM")
def test_a_command_that_ignores_term_is_killed():
    status, output, elapsed = bounded(
        """bounded 1 perl -e '$| = 1; $SIG{TERM} = "IGNORE"; print "up\\n"; sleep 50'; echo "status $?" """
    )
    assert (status, output) == (0, "up\nstatus 124\n")
    assert elapsed < 20
