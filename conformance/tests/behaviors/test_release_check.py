"""The app's automatic release check at launch meets GitHub's rate limit and asks again with the local GitHub
token (`ngterm-update`'s `metadata_response` and `github_auth`), whatever GitHub then answers.

Only an instrumented build points its check at a stand-in for GitHub's API (`launch(releases_api=...)`, which
`ngterm-update` reads under `cfg(coverage)` alone), so these tests run on the runners' `--coverage` runs only. The
instance gets a fake token and none of the run's."""

from __future__ import annotations

import json
import sys
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler

import pytest
from agterm_conformance import llvm_profiles
from agterm_conformance.loopback import serve
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.skipif(
        llvm_profiles.profile_dir() is None,
        reason="only an instrumented build reads a stand-in releases API (the runners' --coverage)",
    ),
    pytest.mark.skipif(sys.platform == "darwin", reason="no update format fits a private install on macOS"),
]

API_PATH = "/repos/nickalie/ngterm/releases/latest"
FAKE_TOKEN = "conformance-fake-github-token"
# Below every release: the check finds no newer one and downloads nothing.
OLD_RELEASE = {"tag_name": "v0.0.1", "draft": False, "prerelease": False, "assets": []}
CHECK_TIMEOUT = 30.0

Answer = tuple[int, dict[str, str], bytes]


def rate_limited() -> Answer:
    reset = str(int(time.time()) + 600)
    headers = {"x-ratelimit-remaining": "0", "x-ratelimit-reset": reset}
    return 403, headers, b'{"message": "API rate limit exceeded"}'


def release() -> Answer:
    return 200, {"Content-Type": "application/json"}, json.dumps(OLD_RELEASE).encode()


def bad_credentials() -> Answer:
    return 401, {}, b'{"message": "Bad credentials"}'


@dataclass(frozen=True)
class Request:
    path: str
    headers: dict[str, str]


class StandIn:
    """GitHub's releases API: it records each request and gives the next answer of `answers`, then a rate limit."""

    def __init__(self, answers: list[Callable[[], Answer]]) -> None:
        self.answers = answers
        self.requests: list[Request] = []
        self.lock = threading.Lock()

    def handler(self) -> type[BaseHTTPRequestHandler]:
        stand_in = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 - http.server's name
                with stand_in.lock:
                    index = len(stand_in.requests)
                    stand_in.requests.append(Request(self.path, {k.lower(): v for k, v in self.headers.items()}))
                status, headers, body = (stand_in.answers[index] if index < len(stand_in.answers) else rate_limited)()
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, format: str, *args: object) -> None:  # noqa: A002 - http.server's signature
                pass

        return Handler

    def received(self, count: int) -> list[Request] | None:
        with self.lock:
            return list(self.requests) if len(self.requests) >= count else None


@pytest.mark.parametrize("second", [release, bad_credentials], ids=["token-accepted", "token-refused"])
def test_a_rate_limited_check_asks_again_with_the_local_token(launch, second):
    stand_in = StandIn([rate_limited, second])
    with serve(stand_in.handler()) as server:
        launch(
            releases_api=f"http://127.0.0.1:{server.server_port}{API_PATH}",
            github_token=FAKE_TOKEN,
        )
        first, retry = wait_until(lambda: stand_in.received(2), timeout=CHECK_TIMEOUT)[:2]

    for request in (first, retry):
        assert request.path == API_PATH
        assert request.headers["accept"] == "application/vnd.github+json"
        assert request.headers["user-agent"].startswith("ngterm/")
    assert "authorization" not in first.headers
    assert retry.headers["authorization"] == f"Bearer {FAKE_TOKEN}"
