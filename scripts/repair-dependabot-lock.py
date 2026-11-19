#!/usr/bin/env python3
"""Repair the Cargo.lock edges a Dependabot cargo update moved, for .github/workflows/dependabot-lock-edges.yml.

Run from a checkout of the base branch (the one this script is in), which is all that is trusted: the pull request's
commit is data. Every mode first refuses a pull request that is not a Dependabot cargo branch whose only difference
from its merge base with the base branch is modified regular files named Cargo.lock or Cargo.toml, so the tree it
then checks out beside this one holds nothing else the base branch does not have.

Usage: scripts/repair-dependabot-lock.py fix HEAD_SHA BRANCH OUT
           run scripts/check-lock-edges.py --fix (this checkout's, with this checkout's cargo config and toolchain)
           over HEAD_SHA's tree and write the repaired Cargo.lock to OUT; no OUT when no edge moved
       scripts/repair-dependabot-lock.py commit HEAD_SHA BRANCH LOCK
           commit LOCK (a file, read as data) as HEAD_SHA's Cargo.lock on top of HEAD_SHA; needs no cargo
       scripts/repair-dependabot-lock.py workflows
           the workflows a Cargo.lock change starts on a pull request, one per line
       scripts/repair-dependabot-lock.py status RUN RUNS
           the commit status, as the JSON body to post, of the commit of RUN (a file: a workflow run as the API gives
           it): the state now of the newest dispatched run of that workflow on that commit, RUN or one in RUNS (a
           file: the API's list of the workflow's runs)
       scripts/repair-dependabot-lock.py --self-test
`fix` and `commit` append `repaired=true` and `commit=SHA` to $GITHUB_OUTPUT when they produced one. Neither pushes.
The self-test needs cargo and PyYAML.
"""

import contextlib
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRANCH_PREFIX = "dependabot/cargo/"
SHA = re.compile(r"^[0-9a-f]{40}$")
MAX_LOCK_BYTES = 8 << 20
# Dependabot rebases and force-pushes over a commit with this marker instead of leaving the pull request alone
MESSAGE = "build(deps): re-point the Cargo.lock edges the update moved\n\n[dependabot skip]\n"
IDENTITY = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com"]
# a push with GITHUB_TOKEN starts no workflow, so the repair workflow dispatches these on the repaired branch, and
# dependabot-lock-checks.yml reports their runs on the commit
WORKFLOWS = ["android.yml", "conformance-gate.yml", "cross-platform.yml", "ios.yml", "linux.yml"]


class Refused(Exception):
    """The pull request is not one this script repairs."""


def git(repo, *args, env=None):
    command = ["git", "-C", str(repo), *args]
    return subprocess.run(command, check=True, stdout=subprocess.PIPE, text=True, env=env and {**os.environ, **env}).stdout


def gate(repo, head, branch):
    if not branch.startswith(BRANCH_PREFIX) or len(branch) == len(BRANCH_PREFIX):
        raise Refused(f"{branch!r} is not a Dependabot cargo branch")
    if not SHA.match(head):
        raise Refused(f"{head!r} is not a commit id")
    base = git(repo, "merge-base", "HEAD", head).strip()
    fields = git(repo, "diff", "--raw", "--no-renames", "--no-abbrev", "-z", base, head).split("\0")[:-1]
    for meta, path in zip(fields[::2], fields[1::2]):
        old_mode, new_mode, _, _, status = meta.lstrip(":").split(" ")
        named = path == "Cargo.lock" or path == "Cargo.toml" or path.endswith("/Cargo.toml")
        if not (named and status == "M" and old_mode == new_mode == "100644"):
            raise Refused(f"the pull request changes more than Cargo.lock and Cargo.toml files: {status} {path}")


@contextlib.contextmanager
def checkout(repo, head):
    with tempfile.TemporaryDirectory(prefix="dependabot-lock-") as parent:
        tree = Path(parent) / "tree"
        git(repo, "worktree", "add", "--quiet", "--detach", str(tree), head)
        try:
            yield tree
        finally:
            git(repo, "worktree", "remove", "--force", str(tree))


def only_the_lock_changed(tree):
    status = git(tree, "status", "--porcelain", "--untracked-files=all")
    if status not in ("", " M Cargo.lock\n"):
        raise RuntimeError(f"more than Cargo.lock changed:\n{status}")
    return bool(status)


def run_fix(repo, tree):
    script = Path(repo) / "scripts" / "check-lock-edges.py"
    subprocess.run([sys.executable, str(script), "--fix", "--manifest-path", str(tree / "Cargo.toml")], check=True)


def fix(repo, head, branch, out, run=run_fix):
    """Whether an edge had moved; the repaired lock is then at `out`."""
    gate(repo, head, branch)
    with checkout(repo, head) as tree:
        run(repo, tree)
        if not only_the_lock_changed(tree):
            return False
        Path(out).write_bytes((tree / "Cargo.lock").read_bytes())
        return True


def read_lock(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise RuntimeError(f"{path} is not a regular file")
    if path.stat().st_size > MAX_LOCK_BYTES:
        raise RuntimeError(f"{path} is too large for a Cargo.lock")
    data = path.read_bytes()
    packages = tomllib.loads(data.decode()).get("package")
    if not isinstance(packages, list) or not packages:
        raise RuntimeError(f"{path} is not a Cargo.lock")
    return data


def commit(repo, head, branch, lock):
    """The commit that puts `lock` on top of `head`, or None when it is `head`'s lock already."""
    gate(repo, head, branch)
    data = read_lock(lock)
    with checkout(repo, head) as tree:
        (tree / "Cargo.lock").write_bytes(data)
        if not only_the_lock_changed(tree):
            return None
        # the same commit id on a rerun: its push is then a no-op, not a rejected sibling of the commit already there
        when = git(repo, "log", "-1", "--date=raw", "--format=%cd", head).strip()
        dates = {"GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when}
        git(tree, *IDENTITY, "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", MESSAGE, "--", "Cargo.lock", env=dates)
        return git(tree, "rev-parse", "HEAD").strip()


def output(line):
    print(line)
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as file:
            file.write(line + "\n")


def status_state(status, conclusion):
    """The commit status state for a workflow run in `status` (the API's, or a `workflow_run` event's action)."""
    if status != "completed":
        return "pending"
    if conclusion in ("success", "neutral", "skipped"):
        return "success"
    return "error" if conclusion in ("cancelled", "stale") else "failure"


def commit_status(run, listed):
    """The commit status that reports `run`'s workflow on `run`'s commit.

    It is the state of the newest dispatched run of that workflow on that commit, not of `run` itself: every event
    of every such run then posts the same status, so neither their order nor a superseded run's late end matters. A
    re-run keeps its run's number, and the API gives its last attempt.
    """
    same = ("workflow_id", "head_sha", "event")
    runs = [other for other in listed if all(other[key] == run[key] for key in same)] + [run]
    newest = max(runs, key=lambda other: other["run_number"])  # `listed` was read after `run`: on a tie it wins
    result = newest["conclusion"] or "" if newest["status"] == "completed" else ""
    return {"state": status_state(newest["status"], result), "context": run["name"], "target_url": newest["html_url"],
            "description": f"{result or 'running'} (dispatched on the repaired Cargo.lock)"}


def path_pattern(pattern):
    if re.search(r"[?+\[\]\\]", pattern):
        raise ValueError(f"a path filter this script does not read: {pattern!r}")
    parts = re.split(r"(\*\*/|\*\*|\*)", pattern)
    wild = {"**/": "(?:.*/)?", "**": ".*", "*": "[^/]*"}
    return re.compile("".join(wild.get(part) or re.escape(part) for part in parts))


def triggers(workflow):
    """A parsed workflow's `on` as {event: its settings or None}."""
    # YAML 1.1 reads the key `on` as the boolean
    on = workflow.get(True, workflow.get("on"))
    if isinstance(on, str):
        return {on: None}
    if isinstance(on, list):
        return dict.fromkeys(on)
    return on or {}


def starts_on(trigger, path):
    """Whether a pull_request trigger with these settings starts its workflow when only `path` changed."""
    trigger = trigger or {}
    if "paths" in trigger:
        started = False
        for pattern in trigger["paths"]:
            if path_pattern(pattern.removeprefix("!")).fullmatch(path):
                started = not pattern.startswith("!")
        return started
    return not any(path_pattern(pattern).fullmatch(path) for pattern in trigger.get("paths-ignore", []))


def lock_workflows(directory):
    """The workflows a Cargo.lock change starts on a pull request: (file, name, whether it can be dispatched)."""
    import yaml

    found = []
    for path in sorted(p for p in Path(directory).iterdir() if p.suffix in (".yml", ".yaml")):
        workflow = yaml.safe_load(path.read_text())
        on = triggers(workflow)
        if "pull_request" in on and starts_on(on["pull_request"], "Cargo.lock"):
            found.append((path.name, workflow.get("name"), "workflow_dispatch" in on))
    return found


def reported_runs(directory):
    """dependabot-lock-checks.yml: its `workflow_run` trigger (the workflows and events it reports) and its group."""
    import yaml

    workflow = yaml.safe_load((Path(directory) / "dependabot-lock-checks.yml").read_text())
    return triggers(workflow)["workflow_run"], workflow["concurrency"]["group"]


PINNED = re.compile(r"@[0-9a-f]{40}$")
GATE = ("github.event.pull_request.user.login == 'dependabot[bot]'",
        "github.event.pull_request.head.repo.full_name == github.repository",
        "startsWith(github.event.pull_request.head.ref, 'dependabot/cargo/')")


def privileged_faults(text, repair):
    """What a workflow that runs with the base branch's rights (the repair, or the one that reports its runs) must
    not be: `text` is its file, `repair` whether it is dependabot-lock-edges.yml. No fault is the empty list."""
    import yaml

    workflow = yaml.safe_load(text)
    faults = []
    if workflow.get("permissions") != {}:
        faults.append("the top-level permissions are not {}")
    if "secrets." in text:
        faults.append("it reads a secret")
    for name, job in workflow["jobs"].items():
        if "environment" in job:
            faults.append(f"{name} runs in an environment")
        for step in job["steps"]:
            uses, settings = step.get("uses", ""), step.get("with") or {}
            if uses and not PINNED.search(uses):
                faults.append(f"{name} uses {uses}, not a commit")
            if "cache" in uses:
                faults.append(f"{name} uses a cache: {uses}")
            # the base branch alone is checked out, and its checkout keeps no credential
            if uses.startswith("actions/checkout@") and (settings.get("persist-credentials") is not False or {"ref", "repository"} & set(settings)):
                faults.append(f"{name} checks out another ref, or keeps the token in .git")
            # an expression in a script is the event's text run as shell: a value reaches a script as an environment variable
            if "${{" in step.get("run", ""):
                faults.append(f"{name} has an expression in a run step")
    if repair:
        if triggers(workflow) != {"pull_request_target": {"types": ["opened", "synchronize", "reopened"]}}:
            faults.append("it starts on something else than a pull request's commits")
        fix, push = workflow["jobs"]["fix"], workflow["jobs"]["push"]
        if not all(part in fix.get("if", "") for part in GATE) or "||" in fix.get("if", ""):
            faults.append("fix does not hold the pull request to Dependabot's own cargo branch of this repository")
        if push.get("needs") != "fix" or push.get("if") != "needs.fix.outputs.repaired == 'true'":
            faults.append("push runs without fix")
        # the job that gives the pull request's manifests and the crates they name to cargo can write nothing
        if fix.get("permissions") != {"contents": "read"} or "github.token" in str(fix) or "GITHUB_TOKEN" in str(fix):
            faults.append("fix holds more than a read-only token it never uses")
        if any("cargo" in str(step) and "repair-dependabot-lock.py commit" not in str(step) for step in push["steps"]):
            faults.append("push runs cargo")
    return faults


def dispatched_faults(text):
    """What a workflow the repair dispatches must not be: its run builds what a Dependabot update pulled in (build
    scripts, proc macros) as a `workflow_dispatch` run, which has the secrets and the token rights the workflow asks
    for, not the read-only token and Dependabot secrets of Dependabot's own `pull_request` run."""
    import yaml

    workflow = yaml.safe_load(text)
    faults = ["it reads a secret"] if "secrets." in text else []
    if workflow.get("permissions") != {"contents": "read"}:
        faults.append("the top-level permissions are not contents: read")
    for name, job in workflow["jobs"].items():
        if "environment" in job:
            faults.append(f"{name} runs in an environment")
        asked = job.get("permissions") or {}
        if asked != "read-all" and (isinstance(asked, str) or any(access not in ("read", "none") for access in asked.values())):
            faults.append(f"{name} asks for more than read access")
    return faults


LOCK_TEXT = 'version = 4\n\n[[package]]\nname = "app"\nversion = "0.1.0"\ndependencies = [\n "sys {}",\n]\n'


def fixture(parent):
    """A base checkout on main and the commit of a Dependabot branch that moved the lock's edge."""
    repo = Path(parent) / "repo"
    repo.mkdir()
    git(repo, "init", "--quiet", "--initial-branch=main")
    (repo / "Cargo.toml").write_text('[package]\nname = "app"\nversion = "0.1.0"\n')
    (repo / "Cargo.lock").write_text(LOCK_TEXT.format("0.61.2"))
    (repo / "src").mkdir()
    (repo / "src" / "lib.rs").write_text("")
    snapshot(repo, "base")
    return repo


FIXTURE_DATE = "2020-01-02T03:04:05+00:00"


def snapshot(repo, message):
    git(repo, "add", "--all")
    dates = {"GIT_AUTHOR_DATE": FIXTURE_DATE, "GIT_COMMITTER_DATE": FIXTURE_DATE}
    git(repo, *IDENTITY, "-c", "commit.gpgsign=false", "commit", "--quiet", "--allow-empty", "-m", message, env=dates)
    return git(repo, "rev-parse", "HEAD").strip()


def branch_commit(repo, change, message="bump"):
    """A commit on a new branch off main that `change` made; main stays checked out."""
    git(repo, "checkout", "--quiet", "-b", "work", "main")
    change(repo)
    head = snapshot(repo, message)
    git(repo, "checkout", "--quiet", "main")
    git(repo, "branch", "--quiet", "-D", "work")
    return head


def restore_edge(_repo, tree):
    (tree / "Cargo.lock").write_text(LOCK_TEXT.format("0.61.2"))


REGISTRY = 'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
CARGO_LOCK_TEXT = (
    "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n"
    '\n[[package]]\nname = "app"\nversion = "0.1.0"\ndependencies = [\n "ranged",\n "strict",\n "sys 0.60.2",\n]\n'
    f'\n[[package]]\nname = "ranged"\nversion = "1.0.0"\n{REGISTRY}dependencies = [\n "sys {{}}",\n]\n'
    f'\n[[package]]\nname = "strict"\nversion = "1.0.0"\n{REGISTRY}dependencies = [\n "sys 0.61.2",\n]\n'
    f'\n[[package]]\nname = "sys"\nversion = "0.60.2"\n{REGISTRY}'
    f'\n[[package]]\nname = "sys"\nversion = "0.61.2"\n{REGISTRY}'
)


def cargo_fixture(parent):
    """A base checkout whose workspace resolves offline, and the cargo home that makes it: crates.io is a directory."""
    vendor = Path(parent) / "vendor"
    crates = {
        "sys-0.60.2": "", "sys-0.61.2": "",
        "ranged-1.0.0": '[dependencies]\nsys = ">=0.52, <0.62"\n', "strict-1.0.0": '[dependencies]\nsys = "0.61"\n',
    }
    for crate, dependencies in crates.items():
        name, version = crate.rsplit("-", 1)
        (vendor / crate / "src").mkdir(parents=True)
        (vendor / crate / "src" / "lib.rs").write_text("")
        (vendor / crate / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n{dependencies}')
        (vendor / crate / ".cargo-checksum.json").write_text('{"files":{},"package":null}')
    home = Path(parent) / "cargo-home"
    home.mkdir()
    (home / "config.toml").write_text(
        f'[source.crates-io]\nreplace-with = "fixture"\n[source.fixture]\ndirectory = {str(vendor)!r}\n'
    )
    repo = Path(parent) / "workspace"
    (repo / "src").mkdir(parents=True)
    git(repo, "init", "--quiet", "--initial-branch=main")
    (repo / "src" / "lib.rs").write_text("")
    manifest = '[package]\nname = "app"\nversion = "0.1.0"\n[dependencies]\nranged = "1"\nstrict = "1"\nsys = "0.60"\n'
    (repo / "Cargo.toml").write_text(manifest)
    (repo / "Cargo.lock").write_text(CARGO_LOCK_TEXT.format("0.61.2"))
    # what the checked tree must not be able to make cargo run or use: cargo is run from the trusted checkout, so it
    # reads neither. The gate lets no pull request change them, but the tree of an old merge base is not vetted.
    ran = Path(parent) / "ran"
    hostile = Path(parent) / "rustc"
    hostile.write_text(f"#!/bin/sh\ntouch {str(ran)!r}\nexit 1\n")
    hostile.chmod(0o755)
    (repo / ".cargo").mkdir()
    (repo / ".cargo" / "config.toml").write_text(f"[build]\nrustc = {str(hostile)!r}\n[alias]\nmetadata = \"no-such-command\"\n")
    (repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "no-such-toolchain"\n')
    snapshot(repo, "base")
    return repo, home


def real_fix_test(parent, branch):
    """`fix` with nothing stubbed: this checkout's check-lock-edges.py and cargo over another repository's commit."""
    repo, home = cargo_fixture(parent)
    out = Path(parent) / "Cargo.lock.cargo"
    moved = branch_commit(repo, lambda r: (r / "Cargo.lock").write_text(CARGO_LOCK_TEXT.format("0.60.2")))
    own_lock = (ROOT / "Cargo.lock").read_bytes()
    kept = os.environ.get("CARGO_HOME")
    os.environ["CARGO_HOME"] = str(home)
    try:
        assert fix(repo, moved, branch, out, lambda _repo, tree: run_fix(ROOT, tree)) is True
        assert out.read_text() == CARGO_LOCK_TEXT.format("0.61.2"), out.read_text()
        out.unlink()
        assert fix(repo, git(repo, "rev-parse", "main").strip(), branch, out, lambda _repo, tree: run_fix(ROOT, tree)) is False
        assert not out.exists()
    finally:
        os.environ.pop("CARGO_HOME") if kept is None else os.environ.update(CARGO_HOME=kept)
    assert (ROOT / "Cargo.lock").read_bytes() == own_lock, "the fix reached this checkout's Cargo.lock"
    assert not (Path(parent) / "ran").exists(), "cargo took its config from the checked tree"
    assert git(repo, "status", "--porcelain") == "" and git(repo, "worktree", "list").count("\n") == 1


def trigger_test():
    lock = lambda trigger: starts_on(trigger, "Cargo.lock")
    everything = ["**", "!**/*.md", "!mobile/ios/**", "resources/**"]
    started = [None, {"branches": ["main"]}, {"paths": ["Cargo.lock"]}, {"paths": ["**/Cargo.lock"]}, {"paths": ["*.lock"]},
               {"paths": everything}, {"paths": ["!Cargo.lock", "Cargo.*"]}, {"paths-ignore": ["**/*.md", "docs/**"]}]
    idle = [{"paths": ["crates/**", "Cargo.toml"]}, {"paths": ["**", "!Cargo.lock"]}, {"paths": ["*/Cargo.lock"]},
            {"paths-ignore": ["*.lock"]}, {"paths-ignore": ["**"]}, {"paths": []}]
    assert all(lock(trigger) for trigger in started), [trigger for trigger in started if not lock(trigger)]
    assert not any(lock(trigger) for trigger in idle), [trigger for trigger in idle if lock(trigger)]
    for unread in ("Cargo.loc?", "Cargo.[a-z]ock", "Cargo+.lock"):
        try:
            lock({"paths": [unread]})
        except ValueError:
            continue
        raise AssertionError(f"read {unread!r} as a plain glob")
    assert triggers({True: ["push", "pull_request"]}) == {"push": None, "pull_request": None}
    assert triggers({True: "pull_request"}) == {"pull_request": None}
    assert triggers({"on": {"pull_request": {"paths": ["a"]}}}) == {"pull_request": {"paths": ["a"]}}

    states = {("requested", ""): "pending", ("in_progress", ""): "pending", ("completed", "success"): "success",
              ("completed", "skipped"): "success", ("completed", "neutral"): "success", ("completed", "failure"): "failure",
              ("completed", "timed_out"): "failure", ("completed", "startup_failure"): "failure",
              ("completed", "action_required"): "failure", ("completed", ""): "failure", ("completed", "cancelled"): "error"}
    assert {case: status_state(*case) for case in states} == states

    def run(number, status, conclusion=None, **other):
        return {"workflow_id": 7, "head_sha": "a" * 40, "event": "workflow_dispatch", "name": "linux", "run_number": number,
                "status": status, "conclusion": conclusion, "html_url": f"https://example.invalid/runs/{number}"} | other

    def posted(event_run, *listed):
        status = commit_status(event_run, list(listed))
        assert status["context"] == "linux"
        return status["state"], status["target_url"][-1], status["description"].split(" ")[0]

    assert posted(run(1, "queued")) == ("pending", "1", "running")
    assert posted(run(1, "completed", "startup_failure")) == ("failure", "1", "startup_failure")
    # the event is behind the run: a late `requested` of a run that has ended, a re-run of one that had passed
    assert posted(run(1, "queued"), run(1, "completed", "failure")) == ("failure", "1", "failure")
    assert posted(run(1, "completed", "success"), run(1, "in_progress", "success")) == ("pending", "1", "running")
    # a superseded run's late end does not replace the status of the run dispatched after it, in either list order
    old, new = run(1, "completed", "cancelled"), run(2, "in_progress")
    assert posted(old, new, old) == posted(old, old, new) == posted(new, old) == ("pending", "2", "running")
    assert posted(old, run(2, "completed", "success")) == ("success", "2", "success")
    # not this commit's, not this workflow's, not dispatched
    others = [run(9, "completed", "success", head_sha="b" * 40), run(9, "completed", "success", workflow_id=8),
              run(9, "completed", "success", event="pull_request")]
    assert posted(run(1, "completed", "failure"), *others) == ("failure", "1", "failure")


def privileged_test(workflows):
    repair = (workflows / "dependabot-lock-edges.yml").read_text()
    checks = (workflows / "dependabot-lock-checks.yml").read_text()
    assert privileged_faults(repair, True) == [], privileged_faults(repair, True)
    assert privileged_faults(checks, False) == [], privileged_faults(checks, False)
    pin = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    # each case breaks one rule and nothing else, and names the fault that rule reports: a case that another rule
    # rejected would pass with its own rule gone
    unsafe = [
        ("permissions: {}", "permissions: write-all", "top-level permissions"),
        (pin, "actions/checkout@v7", "not a commit"),
        ("persist-credentials: false", "persist-credentials: true", "checks out another ref"),
        ("          persist-credentials: false\n", "", "checks out another ref"),
        ("persist-credentials: false", "persist-credentials: false\n          ref: ${{ github.event.pull_request.head.sha }}",
         "checks out another ref"),
        ("persist-credentials: false", "persist-credentials: false\n          repository: someone/fork", "checks out another ref"),
        ('fix "$HEAD_SHA"', "fix ${{ github.event.pull_request.head.ref }}", "expression in a run step"),
        ("GH_TOKEN: ${{ github.token }}", "GH_TOKEN: ${{ secrets.PUSH_TOKEN }}", "reads a secret"),
        ("types: [opened, synchronize, reopened]", "types: [opened, synchronize, reopened, edited]", "starts on something else"),
        ("github.event.pull_request.user.login == 'dependabot[bot]'", "github.actor == 'dependabot[bot]'", "fix does not hold"),
        ("github.event.pull_request.head.repo.full_name == github.repository &&", "", "fix does not hold"),
        ("startsWith(github.event.pull_request.head.ref, 'dependabot/cargo/')", "true", "fix does not hold"),
        ("      contents: read\n    outputs:", "      contents: write\n    outputs:", "fix holds more"),
        ("if: needs.fix.outputs.repaired == 'true'", "if: always()", "push runs without fix"),
        ("    timeout-minutes: 10\n", "    timeout-minutes: 10\n    environment: release\n", "runs in an environment"),
        ("      - uses: actions/download-artifact@", f"      - uses: actions/cache@{'a' * 40}\n      - uses: actions/download-artifact@",
         "uses a cache"),
        ("      - name: commit the repaired lock", "      - run: cargo metadata\n      - name: commit the repaired lock", "push runs cargo"),
    ]
    for old, new, fault in unsafe:
        assert old in repair, f"dependabot-lock-edges.yml no longer has {old!r}: the self-test's cases are stale"
        only(privileged_faults(repair.replace(old, new), True), fault, f"{old!r} as {new!r}")
    only(privileged_faults(checks.replace("COMMIT: ${{", "COMMIT: ${{ secrets.X }} # ${{"), False), "reads a secret", "a secret")
    expression = checks.replace('statuses/$COMMIT"', 'statuses/${{ github.event.workflow_run.head_sha }}"')
    assert expression != checks, "dependabot-lock-checks.yml no longer posts to statuses/$COMMIT: the self-test's case is stale"
    only(privileged_faults(expression, False), "expression in a run step", "an expression in a script")

    for file in WORKFLOWS:
        text = (workflows / file).read_text()
        assert dispatched_faults(text) == [], f"{file}, which the repair dispatches on a Dependabot branch: {dispatched_faults(text)}"
    linux = (workflows / "linux.yml").read_text()
    job = "    runs-on: ubuntu-26.04\n"
    for old, new, fault in [
        ("permissions:\n  contents: read", "permissions:\n  contents: write", "top-level permissions"),
        ("permissions:\n  contents: read", "permissions:\n  contents: read\nenv:\n  KEY: ${{ secrets.SIGNING_KEY }}", "reads a secret"),
        (job, job + "    permissions:\n      id-token: write\n", "more than read access"),
        (job, job + "    permissions: write-all\n", "more than read access"),
        (job, job + "    environment: release\n", "runs in an environment"),
    ]:
        assert old in linux, f"linux.yml no longer has {old!r}: the self-test's cases are stale"
        only(dispatched_faults(linux.replace(old, new)), fault, f"in a dispatched workflow: {new!r}")


def only(faults, fault, case):
    """A broken workflow was refused, and for `fault` alone: every fault reported is the one the case is about."""
    assert faults, f"accepted: {case}"
    assert all(fault in found for found in faults), f"{case}: refused for {faults}, not only for {fault!r}"


def self_test():
    os.environ["GIT_CONFIG_GLOBAL"] = os.environ["GIT_CONFIG_SYSTEM"] = os.devnull
    branch = "dependabot/cargo/sys-0.61.3"
    with tempfile.TemporaryDirectory() as parent:
        repo = fixture(parent)
        out = Path(parent) / "Cargo.lock.repaired"
        moved = branch_commit(repo, lambda r: (r / "Cargo.lock").write_text(LOCK_TEXT.format("0.48.0")))
        (repo / "README").write_text("main moved on\n")
        main = snapshot(repo, "later on main")

        assert fix(repo, moved, branch, out, restore_edge) is True
        assert out.read_text() == LOCK_TEXT.format("0.61.2")
        assert git(repo, "worktree", "list").count("\n") == 1, "the data checkout was left behind"

        repaired = commit(repo, moved, branch, out)
        dated = git(repo, "log", "-1", "--format=%ct", moved).strip()
        assert git(repo, "log", "-1", "--format=%at %ct", repaired) == f"{dated} {dated}\n", "not the same commit on a rerun"
        assert commit(repo, moved, branch, out) == repaired
        assert git(repo, "rev-parse", f"{repaired}^").strip() == moved
        assert git(repo, "diff", "--name-only", moved, repaired) == "Cargo.lock\n"
        assert git(repo, "show", f"{repaired}:Cargo.lock") == LOCK_TEXT.format("0.61.2")
        assert "[dependabot skip]" in git(repo, "log", "-1", "--format=%B", repaired)
        assert git(repo, "log", "-1", "--format=%an", repaired) == "github-actions[bot]\n"
        assert git(repo, "rev-parse", "HEAD").strip() == main, "the base checkout moved"
        assert git(repo, "status", "--porcelain") == ""

        out.unlink()
        assert fix(repo, repaired, branch, out, restore_edge) is False, "a repaired branch was repaired again"
        assert not out.exists()
        assert commit(repo, repaired, branch, repo / "Cargo.lock") is None

        def refused(head, name=branch):
            for call in (lambda: fix(repo, head, name, out, restore_edge), lambda: commit(repo, head, name, repo / "Cargo.lock")):
                try:
                    call()
                except Refused:
                    continue
                raise AssertionError(f"{name} at {head} was not refused")
            assert not out.exists()

        refused(moved, "dependabot/github_actions/actions/checkout-8")
        refused(moved, "dependabot/cargo/")
        refused(moved, "ci/dependabot/cargo/x")
        refused("--upload-pack=x")
        refused(branch_commit(repo, lambda r: (r / "src" / "lib.rs").write_text("fn f() {}\n")))
        refused(branch_commit(repo, lambda r: (r / "build.rs").write_text("fn main() {}\n")))
        refused(branch_commit(repo, lambda r: (r / "sub").mkdir() or (r / "sub" / "Cargo.toml").write_text("[package]\n")))
        refused(branch_commit(repo, lambda r: (r / "Cargo.lock").chmod(0o755)))
        refused(branch_commit(repo, lambda r: (r / "Cargo.lock").unlink() or (r / "Cargo.lock").symlink_to("README")))
        refused(branch_commit(repo, lambda r: (r / "Cargo.toml").unlink()))

        manifest = branch_commit(repo, lambda r: (r / "Cargo.toml").write_text('[package]\nname = "app"\nversion = "0.1.1"\n'))
        assert fix(repo, manifest, branch, out, restore_edge) is False

        def stray(_repo, tree):
            (tree / "Cargo.lock").write_text(LOCK_TEXT.format("0.60.2"))
            (tree / "build.rs").write_text("fn main() {}\n")

        for bad, lock in ((stray, None), (None, Path(parent) / "link"), (None, repo / "Cargo.toml"), (None, repo / "src")):
            (Path(parent) / "link").unlink(missing_ok=True)
            (Path(parent) / "link").symlink_to(repo / "Cargo.lock")
            try:
                fix(repo, moved, branch, out, bad) if bad else commit(repo, moved, branch, lock)
            except RuntimeError:
                assert not out.exists()
                continue
            raise AssertionError(f"accepted: {bad or lock}")
        assert git(repo, "worktree", "list").count("\n") == 1

        real_fix_test(parent, branch)

    trigger_test()
    workflows = ROOT / ".github" / "workflows"
    found = lock_workflows(workflows)
    assert [file for file, _, _ in found] == WORKFLOWS, f"WORKFLOWS is not the workflows a Cargo.lock change starts: {found}"
    assert all(dispatch for _, _, dispatch in found), f"a workflow to dispatch has no workflow_dispatch trigger: {found}"
    reported, group = reported_runs(workflows)
    names = [name for _, name, _ in found]
    assert reported["workflows"] == names, f"dependabot-lock-checks.yml reports {reported['workflows']}, not {found}"
    # a re-run has no `requested` event: without `in_progress` its commit keeps the result of the attempt before
    events = ["requested", "in_progress", "completed"]
    assert reported["types"] == events, f"dependabot-lock-checks.yml reports on {reported['types']}, not {events}"
    # the group is joined before the job's `if`: a run that is not reported must not share a reported one's group
    for part in ("workflow_id", "head_sha", "event"):
        assert f"github.event.workflow_run.{part} " in group, f"dependabot-lock-checks.yml's group has no {part}: {group}"
    privileged_test(workflows)
    print("repair-dependabot-lock: self-test passed")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv == ["workflows"]:
        print("\n".join(WORKFLOWS))
        return 0
    if len(argv) == 3 and argv[0] == "status":
        run, listed = (json.loads(Path(path).read_text()) for path in argv[1:])
        print(json.dumps(commit_status(run, listed["workflow_runs"])))
        return 0
    if len(argv) != 4 or argv[0] not in ("fix", "commit"):
        print(__doc__.strip().split("\n\n")[-1], file=sys.stderr)
        return 2
    mode, head, branch, path = argv
    try:
        if mode == "fix":
            if fix(ROOT, head, branch, path):
                output("repaired=true")
            else:
                print("Cargo.lock: no edge moved, nothing to repair")
        else:
            made = commit(ROOT, head, branch, path)
            if made:
                output(f"commit={made}")
            else:
                print("Cargo.lock: already the repaired one")
    except Refused as reason:
        print(f"not repairing: {reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
