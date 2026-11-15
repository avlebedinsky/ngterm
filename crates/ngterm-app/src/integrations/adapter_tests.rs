//! The Claude and Codex adapter scripts run for real: what the Claude adapter forwards to the shared wrapper for a
//! hook's argv and payload, and which events of either pipe their payload to `ngtermctl agent hook` (the Android chat
//! view's agent record). A stub wrapper records the forwarded argv and a stub `ngtermctl` (`NGTERMCTL`) its argv and
//! stdin; the run's environment holds no `NGTERM_*`, `CLAUDE*` or `CODEX*` variable of the caller's, so nothing
//! reaches an app. The Claude adapter's POSIX script runs on Unix, its PowerShell form (`ngterm-claude-status.ps1`) on
//! Windows, under Windows PowerShell and, where it is installed, PowerShell 7. The Codex adapter is POSIX only: it runs
//! under bash on Unix and under Git for Windows' bash on Windows, as Codex runs it there (`hooks_config::WINDOWS_BASH`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IN_FLIGHT: &str =
    r#"[{"id":"b1","type":"shell","status":"running","description":"cargo test","command":"cargo test"}]"#;

const SOCKET: &str = "/nonexistent/ngterm-adapter-test.sock";
const PANE_ID: &str = "05E0AB65-271B-4389-9946-C3A7E2060CAC";
const CLI: (&str, &str) = ("CLAUDE_CODE_ENTRYPOINT", "cli");

/// A hook payload as Claude Code writes it: compact JSON, the common fields first.
fn payload(event: &str, message: &str, tasks: &str) -> String {
    format!(
        r#"{{"session_id":"s","hook_event_name":"{event}","stop_hook_active":false,"last_assistant_message":"{message}","background_tasks":{tasks},"session_crons":[]}}"#
    )
}

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status")
}

/// A recorded hook payload (`ngterm-core`'s `tests/fixtures/agent_chat`, Task 1 of the chat view's plan).
pub(super) fn fixture(path: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ngterm-core/tests/fixtures/agent_chat");
    fs::read_to_string(dir.join(path)).unwrap()
}

/// Whether `name` is a variable of the caller's that must not reach an adapter: a live instance's, an agent's.
pub(super) fn foreign_var(name: &str) -> bool {
    ["NGTERM_", "AGTERM_", "CLAUDE", "CODEX"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Removes the caller's [`foreign_var`]s from `command`'s environment.
fn scrub(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        if foreign_var(&key) {
            command.env_remove(key);
        }
    }
}

/// `path` as a script under bash finds it: forward slashes, which Git for Windows' bash reads as Windows paths do.
fn bash_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// One `ngtermctl agent hook` call: its arguments and what it read on stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentCall {
    argv: Vec<String>,
    stdin: String,
}

/// A scratch directory with a stub `ngtermctl` that records the argv and stdin of an `agent` call into `agent-argv`
/// (tab-separated) and `agent-stdin`, the argv last.
struct Stub {
    dir: tempfile::TempDir,
}

impl Stub {
    #[cfg(unix)]
    const NAME: &str = "ngtermctl";
    #[cfg(windows)]
    const NAME: &str = "ngtermctl.ps1";

    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self::write_posix(dir.path());
        #[cfg(windows)]
        fs::write(
            dir.path().join(Self::NAME),
            "if ($args[0] -cne 'agent') { exit 0 }\r\n\
             [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'agent-stdin'), (@($input) -join \"`n\"))\r\n\
             [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'agent-argv'), ($args -join \"`t\"))\r\n",
        )
        .unwrap();
        Self { dir }
    }

    /// The POSIX stub, `ngtermctl` in `dir`, which the Codex adapter runs under bash on every OS. An `agent mail` call
    /// is kept apart from the `agent hook` one (`mail-argv`, `mail-stdin`) and answers with `mail-output` when there is
    /// one, as the CLI prints a hook answer when the mailbox held mail.
    fn write_posix(dir: &Path) {
        ngterm_host::fakes::write_executable(
            &dir.join("ngtermctl"),
            "#!/bin/sh\n[ \"$1\" = agent ] || exit 0\ndir=$(dirname \"$0\")\nname=agent\n\
             [ \"$2\" = mail ] && name=mail\ncat > \"$dir/$name-stdin\"\n\
             (IFS='\t'; printf '%s' \"$*\") > \"$dir/$name-argv\"\n\
             if [ \"$name\" = mail ] && [ -f \"$dir/mail-output\" ]; then cat \"$dir/mail-output\"; fi\n",
        )
        .unwrap();
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The `agent` call made since the last [`Self::reset`], if any.
    fn agent_call(&self) -> Option<AgentCall> {
        self.call("agent")
    }

    /// The POSIX stub's `agent mail` call.
    fn mail_call(&self) -> Option<AgentCall> {
        self.call("mail")
    }

    fn call(&self, name: &str) -> Option<AgentCall> {
        let argv = fs::read_to_string(self.path(&format!("{name}-argv"))).ok()?;
        Some(AgentCall {
            argv: argv.split('\t').map(str::to_owned).collect(),
            stdin: fs::read_to_string(self.path(&format!("{name}-stdin"))).unwrap_or_default(),
        })
    }

    #[cfg(windows)]
    fn reset(&self) {
        for name in ["agent-argv", "agent-stdin"] {
            let _ = fs::remove_file(self.path(name));
        }
    }
}

/// What one Claude adapter run did: the argv it forwarded to the wrapper, and its `agent hook` call.
#[derive(Debug)]
struct ClaudeRun {
    forwarded: String,
    agent: Option<AgentCall>,
}

/// The argv the adapter hands the wrapper for `args` with `payload` on stdin.
fn forwarded(args: &[&str], payload: &str) -> String {
    claude(args, payload, &[]).forwarded
}

/// The Claude adapter run for `args` with `payload` on stdin, in a pane's environment from scratch (the session id,
/// the stub, `vars`). The adapter starts only once its launching shell is reaped, so it is an orphan whose ancestor
/// walk never meets the agent that may be running these tests (a second agent above the hook would make it a silent
/// worker).
#[cfg(unix)]
fn claude(args: &[&str], payload: &str, vars: &[(&str, &str)]) -> ClaudeRun {
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done; exec bash "$0" "$@" <"$PAYLOAD" >/dev/null 2>&1) &"#;
    let stub = Stub::new();
    let mut command = Command::new("sh");
    command.args(["-c", launch]);
    claude_orphan(&stub, command, args, payload, vars).expect("the adapter never called the wrapper")
}

/// Runs `command`, which starts the Claude adapter with `args` as an orphan, and waits for its wrapper call; `None`
/// once a `done` file the chain touches appears with no wrapper call.
#[cfg(unix)]
fn claude_orphan(
    stub: &Stub,
    mut command: Command,
    args: &[&str],
    payload: &str,
    vars: &[(&str, &str)],
) -> Option<ClaudeRun> {
    use std::time::{Duration, Instant};

    let wrapper = stub.path("wrapper.sh");
    ngterm_host::fakes::write_executable(
        &wrapper,
        "#!/bin/sh\nprintf '%s' \"$*\" > \"$OUT.tmp\" && mv \"$OUT.tmp\" \"$OUT\"\n",
    )
    .unwrap();
    let (out, done) = (stub.path("argv"), stub.path("done"));
    let input = stub.path("payload.json");
    fs::write(&input, payload).unwrap();
    scrub(&mut command);
    let status = command
        .arg(resources().join("ngterm-claude-status.sh"))
        .args(args)
        .env("PAYLOAD", &input)
        .env("OUT", &out)
        .env("DONE", &done)
        .env("NGTERM_STATUS_WRAPPER", &wrapper)
        .env("NGTERMCTL", stub.path(Stub::NAME))
        .env("NGTERM_SESSION_ID", "S")
        .envs(vars.iter().copied())
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(forwarded) = fs::read_to_string(&out) {
            return Some(ClaudeRun {
                forwarded,
                agent: stub.agent_call(),
            });
        }
        if done.exists() && !out.exists() {
            assert_eq!(stub.agent_call(), None, "a silent hook makes no agent call");
            return None;
        }
        assert!(
            Instant::now() < deadline,
            "the adapter never called the wrapper for {args:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The PowerShell hosts a Windows test runs a script under: Windows PowerShell, and PowerShell 7 when it is on `PATH`.
#[cfg(windows)]
pub(super) fn powershells() -> Vec<&'static str> {
    let pwsh = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"])
        .status()
        .is_ok_and(|status| status.success());
    if pwsh {
        vec!["powershell", "pwsh"]
    } else {
        vec!["powershell"]
    }
}

/// `shell` running the PowerShell script `script` with `args`, as a hook command starts it.
#[cfg(windows)]
pub(super) fn powershell_file(shell: &str, script: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(shell);
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(args);
    command
}

/// The Claude adapter run for `args` with `payload` on stdin, the same under each PowerShell. The adapter runs as the
/// test's child: its ancestor walk meets the test, cargo and their shell, no agent on a CI runner (a developer's agent
/// above them counts once, which still reports).
#[cfg(windows)]
fn claude(args: &[&str], payload: &str, vars: &[(&str, &str)]) -> ClaudeRun {
    let stub = Stub::new();
    let wrapper = stub.path("wrapper.ps1");
    fs::write(
        &wrapper,
        "Set-Content -LiteralPath $env:OUT -Value ($args -join ' ') -NoNewline\r\n",
    )
    .unwrap();
    let out = stub.path("argv");
    let input = stub.path("payload.json");
    fs::write(&input, payload).unwrap();
    let mut seen = Vec::new();
    for shell in powershells() {
        let _ = fs::remove_file(&out);
        stub.reset();
        let mut command = powershell_file(shell, &resources().join("ngterm-claude-status.ps1"), args);
        scrub(&mut command);
        let status = command
            .stdin(fs::File::open(&input).unwrap())
            .env("OUT", &out)
            .env("NGTERM_STATUS_WRAPPER", &wrapper)
            .env("NGTERMCTL", stub.path(Stub::NAME))
            .env("NGTERM_SESSION_ID", "S")
            .envs(vars.iter().copied())
            .status()
            .unwrap();
        assert!(status.success(), "{shell}: {status}");
        let forwarded = fs::read_to_string(&out)
            .unwrap_or_else(|err| panic!("{shell}: the adapter never called the wrapper for {args:?}: {err}"));
        seen.push((forwarded, stub.agent_call()));
    }
    seen.dedup();
    assert_eq!(seen.len(), 1, "the PowerShell hosts differ: {seen:?}");
    let (forwarded, agent) = seen.remove(0);
    ClaudeRun { forwarded, agent }
}

/// The argv of the `agent hook` call a pane's adapter makes for `event`, `tail` addressing the pane.
fn agent_hook(agent: &str, event: &str, tail: &[&str]) -> Vec<String> {
    ["agent", "hook", agent, event, "--target", "S"]
        .into_iter()
        .chain(tail.iter().copied())
        .map(str::to_owned)
        .collect()
}

/// A pane's variables: its role, its token and the socket.
fn pane_vars(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut vars = vec![
        ("NGTERM_PANE", "right"),
        ("NGTERM_PANE_ID", PANE_ID),
        ("NGTERM_SOCKET", SOCKET),
    ];
    vars.extend_from_slice(extra);
    vars
}

/// The Claude adapter's address tail: `--pane`, `--pane-id`, `--socket`, as the restore adapter writes it.
const CLAUDE_TAIL: [&str; 6] = ["--pane", "right", "--pane-id", PANE_ID, "--socket", SOCKET];

/// A Claude `PermissionRequest` whose command runs past the adapter's 4 KiB read of the payload's head, in more than
/// ASCII.
fn long_permission() -> String {
    let command = format!("echo 'привет ✓ {}'", "x".repeat(10_000));
    format!(
        r#"{{"session_id":"s","transcript_path":"/home/u/.claude/projects/-p/s.jsonl","cwd":"/p","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{{"command":"{command}"}}}}"#
    )
}

#[test]
fn a_prompt_and_a_permission_request_pipe_their_payload_to_agent_hook() {
    let vars = pane_vars(&[CLI]);
    let cases = [
        (
            &["active", "--blink"][..],
            "UserPromptSubmit",
            fixture("claude/hooks/02-UserPromptSubmit.json"),
        ),
        (
            &["blocked"],
            "PermissionRequest",
            fixture("claude/hooks/33-PermissionRequest.json"),
        ),
        (&["blocked"], "PermissionRequest", long_permission()),
    ];
    for (args, event, payload) in cases {
        let run = claude(args, &payload, &vars);
        assert_eq!(run.forwarded, args.join(" "), "{event}: the status still goes out");
        let call = run.agent.unwrap_or_else(|| panic!("{event}: no agent hook call"));
        assert_eq!(call.argv, agent_hook("claude", event, &CLAUDE_TAIL), "{event}");
        assert_eq!(call.stdin.trim_end(), payload.trim_end(), "{event}: the payload, whole");
    }
    // without a pane's variables the session alone is addressed
    let prompt = fixture("claude/hooks/02-UserPromptSubmit.json");
    let call = claude(&["active", "--blink"], &prompt, &[CLI]).agent.unwrap();
    assert_eq!(call.argv, agent_hook("claude", "UserPromptSubmit", &[]));
}

/// A pasted prompt of several MiB, which Claude Code writes through a pipe, reaches `agent hook` whole: bash's `read`
/// takes a pipe a byte at a time, so reading the rest with it gave up at its timeout with a fraction of the payload.
#[cfg(unix)]
#[test]
fn a_multi_megabyte_prompt_through_a_pipe_reaches_agent_hook_whole() {
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done; cat "$PAYLOAD" | bash "$0" "$@" >/dev/null 2>&1) &"#;
    let prompt = format!(
        r#"{{"session_id":"s","transcript_path":"/home/u/.claude/projects/-p/s.jsonl","cwd":"/p","hook_event_name":"UserPromptSubmit","prompt":"{}"}}"#,
        "x".repeat(8 << 20)
    );
    let stub = Stub::new();
    let mut command = Command::new("sh");
    command.args(["-c", launch]);
    let run = claude_orphan(&stub, command, &["active", "--blink"], &prompt, &pane_vars(&[CLI]))
        .expect("the adapter never called the wrapper");
    let call = run.agent.expect("no agent hook call");
    assert_eq!(call.argv, agent_hook("claude", "UserPromptSubmit", &CLAUDE_TAIL));
    assert!(
        call.stdin == prompt,
        "the payload arrived cut to {} of {} bytes",
        call.stdin.len(),
        prompt.len()
    );
}

/// `PostToolUse` posts `active` as `UserPromptSubmit` does and `Notification` posts `blocked` as `PermissionRequest`
/// does; neither, nor `Stop`, nor a `SessionEnd`, reaches the agent record through this adapter.
#[test]
fn other_events_make_no_agent_call() {
    let vars = pane_vars(&[CLI]);
    for (args, path) in [
        (&["active", "--blink"][..], "claude/hooks/06-PostToolUse.json"),
        (&["blocked"], "claude/hooks/28-Notification.json"),
        (&["completed", "--auto-reset"], "claude/hooks/03-Stop.json"),
        (&["completed", "--auto-reset"], "claude/hooks/67-SessionEnd.json"),
    ] {
        let run = claude(args, &fixture(path), &vars);
        assert_eq!(run.forwarded, args.join(" "), "{path}");
        assert_eq!(run.agent, None, "{path}");
    }
    // a key quoted in the prompt is escaped, so it is not the payload's event
    let quoted = r#"{"session_id":"s","hook_event_name":"PostToolUse","tool_response":"\"hook_event_name\":\"UserPromptSubmit\""}"#;
    assert_eq!(claude(&["active", "--blink"], quoted, &vars).agent, None);
}

/// A `claude -p` worker (`sdk-cli`), an SDK run or a run with no entrypoint, and a subagent's prompt never report; a
/// subagent's permission prompt does (`agent hook` keeps only the prompt).
#[test]
fn a_worker_or_a_subagents_prompt_makes_no_agent_call() {
    let prompt = fixture("claude/hooks/02-UserPromptSubmit.json");
    for entrypoint in ["sdk-cli", "sdk-ts", "mcp", ""] {
        let vars = pane_vars(&[("CLAUDE_CODE_ENTRYPOINT", entrypoint)]);
        let run = claude(&["active", "--blink"], &prompt, &vars);
        assert_eq!(
            (run.forwarded.as_str(), run.agent),
            ("active --blink", None),
            "{entrypoint}"
        );
    }
    assert_eq!(claude(&["active", "--blink"], &prompt, &pane_vars(&[])).agent, None);
    let vars = pane_vars(&[CLI]);
    let subagent = prompt.replacen(r#""cwd""#, r#""agent_id":"a1b2","agent_type":"Explore","cwd""#, 1);
    assert_eq!(claude(&["active", "--blink"], &subagent, &vars).agent, None);
    let permission =
        fixture("claude/hooks/33-PermissionRequest.json").replacen(r#""cwd""#, r#""agent_id":"a1b2","cwd""#, 1);
    let call = claude(&["blocked"], &permission, &vars)
        .agent
        .expect("a subagent's permission prompt");
    assert_eq!(call.stdin.trim_end(), permission.trim_end());
}

/// The process-topology guard on Unix: bash copies named as agents stand between the orphaned launcher and the
/// adapter. One agent reports, its status and its agent record; a second, different agent above it makes the hook a
/// worker's, silent on both.
#[cfg(unix)]
#[test]
fn a_hook_under_a_second_agent_makes_no_call_at_all() {
    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let names = tempfile::tempdir().unwrap();
    for agent in ["claude", "codex", "omp", "shell"] {
        std::os::unix::fs::symlink(&bash, names.path().join(agent)).unwrap();
    }
    // the chain: <outer> chain.sh <adapter> <args>; chain.sh runs claude -c '<adapter>; :', then touches DONE
    let chain = names.path().join("chain.sh");
    fs::write(
        &chain,
        "\"$INNER\" -c 'bash \"$0\" \"$@\" <\"$PAYLOAD\" >/dev/null 2>&1; :' \"$@\"\n: > \"$DONE\"\n",
    )
    .unwrap();
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done; exec "$OUTER" "$CHAIN" "$0" "$@" >/dev/null 2>&1) &"#;
    let under = |outer: &str| {
        let stub = Stub::new();
        let mut command = Command::new("sh");
        command
            .args(["-c", launch])
            .env("OUTER", names.path().join(outer))
            .env("INNER", names.path().join("claude"))
            .env("CHAIN", &chain);
        let prompt = fixture("claude/hooks/02-UserPromptSubmit.json");
        claude_orphan(&stub, command, &["active", "--blink"], &prompt, &pane_vars(&[CLI]))
    };
    let alone = under("shell").expect("the pane's agent reports");
    assert_eq!(alone.forwarded, "active --blink");
    assert!(alone.agent.is_some(), "and reports its agent record");
    assert!(under("codex").is_none(), "a worker under another agent is silent");
    assert!(under("omp").is_none(), "a Claude worker must not reset the OMP pane");
}

/// The walk ends at the pane's origin, whichever process that is: the app (`ngterm`), a headless origin
/// (`ngterm-headless`) or the session server that holds a live session's shell (`ngterm-session`). An agent above the
/// origin started it (a coding agent's shell that ran `ngterm-headless`, a test harness under an agent) and is not in
/// the pane, so the pane's agent still reports. An agent between the origin and the pane's agent is one in the pane:
/// the hook under it is a worker's and stays silent. Bash copies named as each process stand in the chain.
#[cfg(unix)]
#[test]
fn an_agent_above_the_panes_origin_does_not_silence_the_panes_agent() {
    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let names = tempfile::tempdir().unwrap();
    for name in [
        "claude",
        "codex",
        "shell",
        "ngterm",
        "ngterm-headless",
        "ngterm-session",
    ] {
        std::os::unix::fs::symlink(&bash, names.path().join(name)).unwrap();
    }
    // nest.sh <name>... -- <adapter> <args>: each name runs the rest of the chain as its child, the last the adapter
    let nest = names.path().join("nest.sh");
    fs::write(
        &nest,
        "name=$1\nshift\nif [ \"$name\" = -- ]; then bash \"$@\" <\"$PAYLOAD\" >/dev/null 2>&1\n\
         else \"$NAMES/$name\" \"$NEST\" \"$@\"; fi\n:\n",
    )
    .unwrap();
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done
        "$NAMES/shell" "$NEST" $CHAIN -- "$0" "$@" >/dev/null 2>&1; : > "$DONE") &"#;
    let under = |chain: &str| {
        let stub = Stub::new();
        let mut command = Command::new("sh");
        command
            .args(["-c", launch])
            .env("NAMES", names.path())
            .env("NEST", &nest)
            .env("CHAIN", chain);
        let prompt = fixture("claude/hooks/02-UserPromptSubmit.json");
        claude_orphan(&stub, command, &["active", "--blink"], &prompt, &pane_vars(&[CLI]))
    };
    for origin in [
        "ngterm",
        "ngterm-headless",
        "ngterm-session",
        "ngterm-headless ngterm-session",
        "ngterm ngterm-session",
    ] {
        let run = under(&format!("codex shell {origin} shell claude"))
            .unwrap_or_else(|| panic!("{origin}: the pane's agent is silent under an agent above its origin"));
        assert_eq!(run.forwarded, "active --blink", "{origin}");
        assert!(run.agent.is_some(), "{origin}: and reports its agent record");
        assert!(
            under(&format!("{origin} shell codex shell claude")).is_none(),
            "{origin}: a worker under another agent in the pane is silent"
        );
    }
}

/// The bash the Codex adapter runs under on Windows: Git for Windows' where its installer puts it; `None` on a machine
/// without it.
#[cfg(windows)]
fn codex_bash() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| PathBuf::from(dir).join("Git").join("bin").join("bash.exe"))
        .find(|bash| bash.is_file())
}

/// What one Codex adapter run did: the states it reported through the wrapper, and its `agent hook` call.
#[derive(Debug)]
struct CodexRun {
    statuses: Vec<String>,
    agent: Option<AgentCall>,
    /// Its `agent mail` call, and what it printed for Codex.
    mail: Option<AgentCall>,
    stdout: String,
}

/// How a Codex adapter run differs from a pane's stock one.
#[derive(Default)]
struct CodexSetup {
    /// What the stub's `agent mail` prints: the hook answer for mail the mailbox held.
    mail_output: Option<&'static str>,
    /// Run without `NGTERM_PANE_ID`, as an app from before stable pane ids starts a pane.
    no_pane_id: bool,
    /// Further variables of the run.
    vars: Vec<(&'static str, String)>,
}

/// The Codex adapter run for `action` with `payload` on stdin, in a pane's environment from scratch, its watcher
/// making no check; `None` after printing why on Windows without Git for Windows' bash.
fn codex(action: &str, payload: &str) -> Option<CodexRun> {
    codex_with(action, payload, &CodexSetup::default())
}

#[cfg(windows)]
fn codex_with(action: &str, payload: &str, setup: &CodexSetup) -> Option<CodexRun> {
    use std::io::Write as _;
    use std::process::Stdio;

    let Some(bash) = codex_bash() else {
        eprintln!("SKIPPED: {action}: no Git for Windows bash");
        return None;
    };
    let stub = Stub::new();
    let mut command = Command::new(bash);
    command
        .arg(bash_path(&resources().join("ngterm-codex-status.sh")))
        .arg(action);
    codex_env(&mut command, &stub, setup);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // the adapter may exit without reading it all
    let _ = child.stdin.take().unwrap().write_all(payload.as_bytes());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{action}: {}", output.status);
    Some(codex_run(&stub, String::from_utf8(output.stdout).unwrap()))
}

/// On Unix the adapter runs as the pane's own Codex: under a Codex in a shell of an origin ([`codex_under`]), so its
/// ancestor walk ends there and never meets the agent that may be running these tests.
#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Windows form skips without Git for Windows' bash"
)]
fn codex_with(action: &str, payload: &str, setup: &CodexSetup) -> Option<CodexRun> {
    Some(codex_under("ngterm shell codex", action, payload, setup))
}

/// The stubs and the pane's environment of a Codex adapter run, on `command`: a wrapper that records each status, the
/// stub `ngtermctl`, a watcher that makes no check.
fn codex_env(command: &mut Command, stub: &Stub, setup: &CodexSetup) {
    let (statuses, wrapper) = (stub.path("statuses"), stub.path("status-wrapper"));
    ngterm_host::fakes::write_executable(&wrapper, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$STATUSES\"\n").unwrap();
    if let Some(output) = setup.mail_output {
        fs::write(stub.path("mail-output"), output).unwrap();
    }
    scrub(command);
    command
        .env("STATUSES", bash_path(&statuses))
        .env("NGTERMCTL", bash_path(&stub.path("ngtermctl")))
        .env("NGTERM_STATUS_WRAPPER", bash_path(&wrapper))
        .env("NGTERM_SESSION_ID", "S")
        .envs(pane_vars(&[]))
        .env("NGTERM_CODEX_WATCH_FILE", bash_path(&stub.path("watch")))
        .env("NGTERM_CODEX_WATCH_MAX_CHECKS", "0")
        .env("TMPDIR", bash_path(stub.dir.path()));
    if setup.no_pane_id {
        command.env_remove("NGTERM_PANE_ID");
    }
    for (name, value) in &setup.vars {
        command.env(name, value);
    }
}

/// What a finished Codex adapter run left in `stub`, with what it printed.
fn codex_run(stub: &Stub, stdout: String) -> CodexRun {
    CodexRun {
        statuses: fs::read_to_string(stub.path("statuses"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
        agent: stub.agent_call(),
        mail: stub.mail_call(),
        stdout,
    }
}

/// The Codex adapter run for `action` at the bottom of a process chain ([`run_under`]).
#[cfg(unix)]
fn codex_under(chain: &str, action: &str, payload: &str, setup: &CodexSetup) -> CodexRun {
    let stub = Stub::new();
    let mut command = Command::new("sh");
    codex_env(&mut command, &stub, setup);
    let script = resources().join("ngterm-codex-status.sh");
    let stdout = run_under(&stub, command, chain, &script, &[action], payload);
    codex_run(&stub, stdout)
}

/// Runs `script` with `args` and `payload` on stdin at the bottom of a process chain, in the environment `command`
/// holds, and returns what it printed. The chain is bash copies named as each word of `chain`, each the child of the
/// one before, under an orphaned launcher, so an ancestor walk ends above the chain's first process and never meets
/// the agent that may be running these tests. `codex` stands for an interactive Codex and `codex-exec` for a `codex
/// exec` run (argv `codex exec ...`); `codex-resume` and `codex-exec-resume` put `resume` after those (the second
/// with a session id), `codex-exec-resume-last` has `--last` in the id's place, `codex-exec-resume-prompt` is `codex
/// exec resume` with no session id, `codex-exec-resume-words` has the id and `--last` behind a first positional that
/// is neither (`codex exec resume of <id> --last`: the words of a TUI prompt, or of a thread name and its prompt),
/// `codex-dashed` has an option and `--` before its first positional (`codex --norc -- ...`), and
/// `codex-spaced-exec` is a `codex exec` run behind an option whose value has a space (`codex -C "<dir>/My Project"
/// exec ...`).
#[cfg(unix)]
fn run_under(stub: &Stub, mut command: Command, chain: &str, script: &Path, args: &[&str], payload: &str) -> String {
    use std::time::{Duration, Instant};

    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let names = stub.path("names");
    fs::create_dir(&names).unwrap();
    for name in [
        "claude",
        "codex",
        "pi",
        "opencode",
        "shell",
        "ngterm",
        "ngterm-headless",
        "ngterm-session",
    ] {
        std::os::unix::fs::symlink(&bash, names.join(name)).unwrap();
    }
    // nest.sh <name>... -- <script> <args>: each name runs the rest of the chain as its child, the last the script.
    // The copies named `exec`, `resume` and `My Project` are the script a Codex stand-in runs, from the names'
    // directory: bash takes the stand-in's first word that is no option as its script, and that script drops the
    // subcommand words and what stands after them (the session id, `--last`, `of`). The stand-in's `-C` is bash's
    // noclobber, which the script turns off again.
    let nest = names.join("nest.sh");
    let nest_script = "set +C\nID=0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b\nwhile :; do case \"$1\" in exec | resume | of | --last | $ID) shift ;; *) break ;; esac; done\n\
        name=$1\nshift\ncase \"$name\" in\n\
        --) bash \"$@\" <\"$PAYLOAD\" >\"$STDOUT\" 2>/dev/null; echo $? >\"$EXIT\" ;;\n\
        codex-exec) cd \"$NAMES\" && \"$NAMES/codex\" exec \"$@\" ;;\n\
        codex-resume) cd \"$NAMES\" && \"$NAMES/codex\" resume \"$@\" ;;\n\
        codex-exec-resume) cd \"$NAMES\" && \"$NAMES/codex\" exec resume $ID \"$@\" ;;\n\
        codex-exec-resume-last) cd \"$NAMES\" && \"$NAMES/codex\" exec resume --last \"$@\" ;;\n\
        codex-exec-resume-words) cd \"$NAMES\" && \"$NAMES/codex\" exec resume of $ID --last \"$@\" ;;\n\
        codex-exec-resume-prompt) cd \"$NAMES\" && \"$NAMES/codex\" exec resume \"$@\" ;;\n\
        codex-dashed) \"$NAMES/codex\" --norc -- \"$NEST\" \"$@\" ;;\n\
        codex-spaced-exec) \"$NAMES/codex\" -C \"$NAMES/My Project\" exec \"$@\" ;;\n\
        *) \"$NAMES/$name\" \"$NEST\" \"$@\" ;;\nesac\n:\n";
    fs::write(&nest, nest_script).unwrap();
    for copy in ["exec", "resume", "My Project"] {
        fs::write(names.join(copy), nest_script).unwrap();
    }
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done
        "$NAMES/shell" "$NEST" $CHAIN -- "$0" "$@" >/dev/null 2>&1; : > "$DONE") &"#;
    let (input, stdout, exit, done) = (
        stub.path("payload.json"),
        stub.path("stdout"),
        stub.path("exit"),
        stub.path("done"),
    );
    fs::write(&input, payload).unwrap();
    let status = command
        .args(["-c", launch])
        .arg(script)
        .args(args)
        .env("NAMES", &names)
        .env("NEST", &nest)
        .env("CHAIN", chain)
        .env("PAYLOAD", &input)
        .env("STDOUT", &stdout)
        .env("EXIT", &exit)
        .env("DONE", &done)
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done.exists() {
        assert!(Instant::now() < deadline, "{chain}: {args:?}: the chain never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read_to_string(&exit).unwrap().trim(),
        "0",
        "{chain}: {args:?}: the script's exit status"
    );
    fs::read_to_string(&stdout).unwrap()
}

/// The Codex adapter's address tail: `--socket`, then `--pane-id` and `--pane`, as it orders them for `session text`.
const CODEX_TAIL: [&str; 6] = ["--socket", SOCKET, "--pane-id", PANE_ID, "--pane", "right"];

#[test]
fn codex_start_prompt_and_permission_request_pipe_their_payload_to_agent_hook() {
    let cases = [
        (
            "session-start",
            "SessionStart",
            "codex/hooks/main/01-SessionStart.json",
            &["idle"][..],
        ),
        (
            "user-prompt-submit",
            "UserPromptSubmit",
            "codex/hooks/main/02-UserPromptSubmit.json",
            &["active --blink"],
        ),
        // a candidate only: the watcher, not this event, reports blocked
        (
            "permission-request",
            "PermissionRequest",
            "codex/hooks/main/25-PermissionRequest.json",
            &[],
        ),
    ];
    for (action, event, path, statuses) in cases {
        let payload = fixture(path);
        let Some(run) = codex(action, &payload) else {
            return;
        };
        assert_eq!(run.statuses, statuses, "{action}: the status is upstream's");
        let call = run.agent.unwrap_or_else(|| panic!("{action}: no agent hook call"));
        assert_eq!(call.argv, agent_hook("codex", event, &CODEX_TAIL), "{action}");
        assert_eq!(
            call.stdin.trim_end(),
            payload.trim_end(),
            "{action}: the payload, whole"
        );
    }
}

#[test]
fn codex_tool_and_stop_events_make_no_agent_call() {
    for (action, path, status) in [
        ("pre-tool-use", "codex/hooks/main/05-PreToolUse.json", "active --blink"),
        (
            "post-tool-use",
            "codex/hooks/main/06-PostToolUse.json",
            "active --blink",
        ),
        ("stop", "codex/hooks/main/03-Stop.json", "completed --auto-reset"),
    ] {
        let Some(run) = codex(action, &fixture(path)) else {
            return;
        };
        assert_eq!((run.statuses, run.agent), (vec![status.to_owned()], None), "{action}");
    }
}

/// The mail tail: `ngtermctl agent mail codex` with the adapter's `--socket`.
const CODEX_MAIL: [&str; 5] = ["agent", "mail", "codex", "--socket", SOCKET];

const BLOCK: &str = r#"{"decision":"block","reason":"ngterm mail: rebase first"}"#;

#[test]
fn codex_hands_the_panes_mail_over_on_prompt_tool_and_stop() {
    let context = r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"ngterm mail: hi"}}"#;
    for (action, path, output, statuses) in [
        (
            "user-prompt-submit",
            "codex/hooks/main/02-UserPromptSubmit.json",
            context,
            &["active --blink"][..],
        ),
        (
            "post-tool-use",
            "codex/hooks/main/06-PostToolUse.json",
            context,
            &["active --blink"],
        ),
        // the turn goes on with the mail, so the row stays active instead of completing
        ("stop", "codex/hooks/main/03-Stop.json", BLOCK, &["active --blink"]),
    ] {
        let payload = fixture(path);
        let setup = CodexSetup {
            mail_output: Some(output),
            ..CodexSetup::default()
        };
        let Some(run) = codex_with(action, &payload, &setup) else {
            return;
        };
        let call = run.mail.unwrap_or_else(|| panic!("{action}: no agent mail call"));
        assert_eq!(call.argv, CODEX_MAIL, "{action}");
        assert_eq!(
            call.stdin.trim_end(),
            payload.trim_end(),
            "{action}: the payload, whole"
        );
        assert_eq!(run.stdout.trim_end(), output, "{action}: the answer, for Codex");
        assert_eq!(run.statuses, statuses, "{action}");
    }
}

#[test]
fn codex_without_mail_prints_nothing_and_ends_its_turn_as_before() {
    for action in ["post-tool-use", "stop"] {
        let path = if action == "stop" {
            "codex/hooks/main/03-Stop.json"
        } else {
            "codex/hooks/main/06-PostToolUse.json"
        };
        let Some(run) = codex(action, &fixture(path)) else {
            return;
        };
        assert!(run.mail.is_some(), "{action}: the mailbox is read");
        assert_eq!(run.stdout, "", "{action}");
    }
}

/// Which Codex run takes mail (its TUI, not a `codex exec` worker, a subagent or the memory-consolidation thread) is
/// `agent mail codex`'s to decide from the payload, so the adapter hands every payload over when the pane has an id.
#[test]
fn codex_takes_no_mail_without_a_pane_id() {
    let setup = CodexSetup {
        mail_output: Some(BLOCK),
        no_pane_id: true,
        ..CodexSetup::default()
    };
    let Some(run) = codex_with("stop", &fixture("codex/hooks/main/03-Stop.json"), &setup) else {
        return;
    };
    assert_eq!((run.mail, run.stdout.as_str()), (None, ""));
    assert_eq!(run.statuses, ["completed --auto-reset"]);
}

/// The Claude mail adapter hands its process over to `ngtermctl agent mail` (so a waiter's parent is the agent that
/// started the hook, and a waiter notices that agent's exit), with the pane's socket, the payload and the exit status
/// passing through untouched.
#[cfg(unix)]
#[test]
fn the_claude_mail_adapter_hands_its_process_to_ngtermctl() {
    let stub = Stub::new();
    let ctl = stub.path("mail-ctl");
    ngterm_host::fakes::write_executable(
        &ctl,
        "#!/bin/sh
printf '%s' \"$PPID\" > \"$OUT/ppid\"\n(IFS='\t'; printf '%s' \"$*\") > \"$OUT/argv\"\n\
         cat > \"$OUT/stdin\"\necho mail >&2\nexit 2\n",
    )
    .unwrap();
    let mut command = Command::new("bash");
    scrub(&mut command);
    let output = command
        .arg(resources().join("ngterm-agent-mail.sh"))
        .args(["claude", "--wait"])
        .env("OUT", stub.dir.path())
        .env("NGTERMCTL", &ctl)
        .env("NGTERM_SESSION_ID", "S")
        .envs(pane_vars(&[]))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"{\"hook_event_name\":\"Stop\"}")?;
            child.wait_with_output()
        })
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "mail\n");
    assert_eq!(
        fs::read_to_string(stub.path("ppid")).unwrap(),
        std::process::id().to_string(),
        "ngtermctl's parent is the adapter's"
    );
    assert_eq!(
        fs::read_to_string(stub.path("argv")).unwrap(),
        format!("agent\tmail\tclaude\t--wait\t--socket\t{SOCKET}")
    );
    assert_eq!(
        fs::read_to_string(stub.path("stdin")).unwrap(),
        r#"{"hook_event_name":"Stop"}"#
    );
}

/// Codex fires a subagent's events in the parent with its `agent_id` and the subagent's transcript: its prompt never
/// re-points the record, its permission prompt is reported.
#[test]
fn a_codex_subagents_prompt_makes_no_agent_call() {
    let Some(run) = codex(
        "user-prompt-submit",
        &fixture("codex/hooks/main/18-UserPromptSubmit.json"),
    ) else {
        return;
    };
    assert_eq!((run.statuses, run.agent), (vec!["active --blink".to_owned()], None));
    let permission = fixture("codex/hooks/main/25-PermissionRequest.json").replacen(
        r#""transcript_path""#,
        r#""agent_id":"01a1a301-8737-70e2-bc57-72fa1f1f57c7","transcript_path""#,
        1,
    );
    let Some(run) = codex("permission-request", &permission) else {
        return;
    };
    let call = run.agent.expect("a subagent's permission prompt");
    assert_eq!(call.argv, agent_hook("codex", "PermissionRequest", &CODEX_TAIL));
}

/// A Stop payload whose last message asks a question, which a pane's Codex reports as `blocked`; `rollout` is its
/// `transcript_path`.
#[cfg(unix)]
fn codex_question(rollout: &Path) -> String {
    format!(
        r#"{{"session_id":"s","transcript_path":"{}","hook_event_name":"Stop","last_assistant_message":"Shall I go on?"}}"#,
        rollout.display()
    )
}

/// Every event of a Codex run that is silent: no status, no agent record, no mail taken.
#[cfg(unix)]
fn assert_codex_silent(chain: &str, setup: &CodexSetup) {
    let question = codex_question(Path::new("/nonexistent/rollout.jsonl"));
    for (action, payload) in [
        ("session-start", fixture("codex/hooks/main/01-SessionStart.json")),
        (
            "user-prompt-submit",
            fixture("codex/hooks/main/02-UserPromptSubmit.json"),
        ),
        ("pre-tool-use", fixture("codex/hooks/main/05-PreToolUse.json")),
        ("post-tool-use", fixture("codex/hooks/main/06-PostToolUse.json")),
        (
            "permission-request",
            fixture("codex/hooks/main/25-PermissionRequest.json"),
        ),
        ("stop", question),
    ] {
        let run = codex_under(chain, action, &payload, setup);
        assert_eq!(run.statuses, Vec::<String>::new(), "{chain}: {action}: a status");
        assert_eq!(run.agent, None, "{chain}: {action}: an agent record");
        assert_eq!(run.mail, None, "{chain}: {action}: mail taken");
        assert_eq!(run.stdout, "", "{chain}: {action}");
    }
}

/// A `codex exec` run that a review tool started under a Claude Code worker inherits that worker's `NGTERM_*`: its
/// Stop, with a "?" in the review, painted the worker's session `blocked`. A Codex with another agent above it inside
/// the pane is a worker's and reports nothing, on any event.
#[cfg(unix)]
#[test]
fn a_codex_under_another_agent_in_the_pane_reports_nothing() {
    let setup = CodexSetup {
        mail_output: Some("mail"),
        ..CodexSetup::default()
    };
    assert_codex_silent("ngterm shell claude shell codex-exec", &setup);
    assert_codex_silent("ngterm shell claude shell codex", &setup);
}

/// A `codex exec` run has no user at its end, whoever started it: it reports nothing even alone in a pane.
#[cfg(unix)]
#[test]
fn a_codex_exec_run_reports_nothing() {
    let setup = CodexSetup {
        mail_output: Some("mail"),
        ..CodexSetup::default()
    };
    assert_codex_silent("ngterm shell codex-exec", &setup);
    // the exact argv tells it: by the words `ps` joined its `-C` value splits, and with no rollout they stand
    if Path::new("/proc/self/cmdline").exists() {
        assert_codex_silent("ngterm shell codex-spaced-exec", &setup);
    }
    assert_codex_silent("ngterm shell codex-exec-resume", &setup);
    assert_codex_silent("ngterm shell codex-exec-resume-last", &setup);
}

/// The walk ends at the pane's origin, as the Claude adapter's does: an agent above the origin started it (a coding
/// agent's shell that ran `ngterm-headless`) and is not in the pane, so the pane's interactive Codex still reports.
#[cfg(unix)]
#[test]
fn an_agent_above_the_panes_origin_does_not_silence_the_panes_codex() {
    let question = codex_question(Path::new("/nonexistent/rollout.jsonl"));
    for origin in [
        "ngterm",
        "ngterm-headless",
        "ngterm-session",
        "ngterm-headless ngterm-session",
        "ngterm ngterm-session",
    ] {
        let chain = format!("claude shell {origin} shell codex");
        let run = codex_under(&chain, "stop", &question, &CodexSetup::default());
        assert_eq!(run.statuses, ["blocked"], "{chain}");
        let chain = format!("{origin} shell claude shell codex");
        let run = codex_under(&chain, "stop", &question, &CodexSetup::default());
        assert_eq!(run.statuses, Vec::<String>::new(), "{chain}");
    }
}

/// Where the Codex's argv cannot be read whole (no `/proc`: macOS), the words `ps` joined say what this run is, but a
/// value with a space splits in them. They decide alone only where a split cannot mislead, whatever the rollout's
/// `session_meta` says, since a session keeps its `source` through a resume: no option before the first positional,
/// or `resume` as that positional, is the TUI and reports (an interactive `codex resume` of a session that `codex
/// exec` created keeps `source: exec`); `exec resume` with `--last` among its options or a session id as its first
/// positional is silent (it keeps a TUI session's `cli`). Other words ask the rollout: `exec` reads the same as a prompt split into words (`codex "exec
/// this"`, `codex "exec resume is failing"`), and a `codex exec` behind an option value with a space reads as the
/// TUI. There `exec` is silent, `cli` (the TUI) reports, and with no rollout the words stand. Still wrong, as the
/// hook's comment says: behind such an option value a resume in either direction and a split that leaves `resume` or
/// `fork` first, a `codex exec resume` of a TUI session by a thread name or with `--last` behind its prompt, and a
/// TUI session whose prompt starts with `exec resume --last` or `exec resume <session id>`.
#[cfg(unix)]
#[test]
fn without_proc_the_joined_argv_tells_a_codex_exec_run_before_the_rollouts_source() {
    let dir = tempfile::tempdir().unwrap();
    let setup = CodexSetup {
        vars: vec![("NGTERM_CODEX_PROC", "/nonexistent".to_owned())],
        ..CodexSetup::default()
    };
    let statuses = |chain: &str, rollout: &Path| codex_under(chain, "stop", &codex_question(rollout), &setup).statuses;
    let none = Vec::<String>::new();
    let (exec, cli) = (codex_rollout(dir.path(), "exec"), codex_rollout(dir.path(), "cli"));
    let missing = Path::new("/nonexistent/rollout.jsonl");
    assert_eq!(
        statuses("ngterm shell codex", &exec),
        ["blocked"],
        "an exec session resumed in the TUI"
    );
    assert_eq!(statuses("ngterm shell codex", &cli), ["blocked"], "a TUI session");
    assert_eq!(statuses("ngterm shell codex", missing), ["blocked"]);
    assert_eq!(statuses("ngterm shell codex-exec", &exec), none, "an exec run");
    assert_eq!(
        statuses("ngterm shell codex-exec", &cli),
        ["blocked"],
        "a TUI session whose prompt starts with `exec`"
    );
    assert_eq!(statuses("ngterm shell codex-exec", missing), none, "the joined argv");
    assert_eq!(
        statuses("ngterm shell codex-spaced-exec", &exec),
        none,
        "an exec run behind an option value with a space"
    );
    assert_eq!(
        statuses("ngterm shell codex-spaced-exec", &cli),
        ["blocked"],
        "a TUI session behind an option value with a space"
    );
    assert_eq!(
        statuses("ngterm shell codex-spaced-exec", missing),
        ["blocked"],
        "words that read as the TUI stand"
    );
    assert_eq!(
        statuses("ngterm shell codex-dashed", &exec),
        none,
        "an exec run behind an option value that holds `--`"
    );
    assert_eq!(
        statuses("ngterm shell codex-dashed", &cli),
        ["blocked"],
        "a TUI session with options and `--` before its prompt"
    );
    assert_eq!(statuses("ngterm shell codex-dashed", missing), ["blocked"]);
    // pre-tool-use reads no payload for anything else
    let run = codex_under(
        "ngterm shell codex-exec",
        "pre-tool-use",
        &codex_question(&exec),
        &setup,
    );
    assert_eq!(run.statuses, none, "pre-tool-use of an exec run");
    let run = codex_under("ngterm shell codex-exec", "pre-tool-use", &codex_question(&cli), &setup);
    assert_eq!(run.statuses, ["active --blink"], "pre-tool-use of a TUI session");
}

/// The same words read a resume: `codex resume` is the TUI whatever the rollout says, `codex exec resume` is silent
/// only with `--last` among its options or a session id as its first positional. Any other words ask the rollout,
/// an id or `--last` further on too: a TUI prompt can start with the same two words and name either later.
#[cfg(unix)]
#[test]
fn without_proc_the_joined_argv_tells_a_resumed_codex_session() {
    let dir = tempfile::tempdir().unwrap();
    let setup = CodexSetup {
        vars: vec![("NGTERM_CODEX_PROC", "/nonexistent".to_owned())],
        ..CodexSetup::default()
    };
    let statuses = |chain: &str, rollout: &Path| codex_under(chain, "stop", &codex_question(rollout), &setup).statuses;
    let none = Vec::<String>::new();
    let (exec, cli) = (codex_rollout(dir.path(), "exec"), codex_rollout(dir.path(), "cli"));
    let missing = Path::new("/nonexistent/rollout.jsonl");
    assert_eq!(
        statuses("ngterm shell codex-resume", &exec),
        ["blocked"],
        "an exec session resumed in the TUI by `codex resume`"
    );
    assert_eq!(
        statuses("ngterm shell codex-exec-resume", &cli),
        none,
        "a TUI session resumed by `codex exec resume <id>`"
    );
    assert_eq!(statuses("ngterm shell codex-exec-resume", missing), none);
    assert_eq!(
        statuses("ngterm shell codex-exec-resume-last", &cli),
        none,
        "a TUI session resumed by `codex exec resume --last`"
    );
    assert_eq!(statuses("ngterm shell codex-exec-resume-last", missing), none);
    assert_eq!(
        statuses("ngterm shell codex-exec-resume-words", &cli),
        ["blocked"],
        "a TUI session whose prompt starts with `exec resume` and names an id and `--last` further on"
    );
    assert_eq!(
        statuses("ngterm shell codex-exec-resume-words", &exec),
        none,
        "an exec session resumed by a thread name"
    );
    assert_eq!(
        statuses("ngterm shell codex-exec-resume-prompt", &cli),
        ["blocked"],
        "a TUI session whose prompt starts with `exec resume`"
    );
    assert_eq!(
        statuses("ngterm shell codex-exec-resume-prompt", &exec),
        none,
        "an exec session resumed by `codex exec resume` with no id in the words"
    );
    assert_eq!(statuses("ngterm shell codex-exec-resume-prompt", missing), none);
}

/// Where there is no process table at all (Git Bash on Windows: here a `ps` that fails), the rollout's `source` alone
/// tells a `codex exec` run, and with no rollout the run reports.
#[cfg(unix)]
#[test]
fn without_a_process_table_the_rollouts_source_tells_a_codex_exec_run() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    ngterm_host::fakes::write_executable(&bin.join("ps"), "#!/bin/sh\nexit 1\n").unwrap();
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let setup = CodexSetup {
        vars: vec![("PATH", path.into_string().unwrap())],
        ..CodexSetup::default()
    };
    let statuses = |chain: &str, rollout: &Path| codex_under(chain, "stop", &codex_question(rollout), &setup).statuses;
    let none = Vec::<String>::new();
    let (exec, cli) = (codex_rollout(dir.path(), "exec"), codex_rollout(dir.path(), "cli"));
    assert_eq!(statuses("ngterm shell codex-exec", &exec), none, "an exec session");
    assert_eq!(statuses("ngterm shell codex", &cli), ["blocked"], "a TUI session");
    let missing = Path::new("/nonexistent/rollout.jsonl");
    assert_eq!(
        statuses("ngterm shell codex-exec", missing),
        ["blocked"],
        "unreadable reports"
    );
    let run = codex_under(
        "ngterm shell codex-exec",
        "pre-tool-use",
        &codex_question(&exec),
        &setup,
    );
    assert_eq!(run.statuses, none, "pre-tool-use of an exec session");
}

/// A rollout in `dir` whose `session_meta` says the session was created by `source`, after a string value that quotes
/// the key.
#[cfg(unix)]
fn codex_rollout(dir: &Path, source: &str) -> PathBuf {
    let path = dir.join(format!("rollout-{source}.jsonl"));
    let meta = format!(
        r#"{{"timestamp":"t","type":"session_meta","payload":{{"id":"s","base_instructions":"a \"source\":\"cli\" inside","source":"{source}","model_provider":"openai"}}}}"#
    );
    fs::write(&path, format!("{meta}\n{{\"type\":\"event_msg\"}}\n")).unwrap();
    path
}

/// The `session status` call the shared wrapper makes for `active --blink` at the bottom of `chain`, as the Pi
/// extension and the `OpenCode` plugins run it; `None` when it makes none.
#[cfg(unix)]
fn wrapper_under(chain: &str, vars: &[(&str, &str)]) -> Option<String> {
    let stub = Stub::new();
    let (ngtermctl, call) = (stub.path("status-ctl"), stub.path("status-call"));
    ngterm_host::fakes::write_executable(&ngtermctl, "#!/bin/sh\nprintf '%s' \"$*\" > \"$CALL\"\n").unwrap();
    let mut command = Command::new("sh");
    scrub(&mut command);
    command
        .env("CALL", &call)
        .env("NGTERMCTL", &ngtermctl)
        .env("NGTERM_SESSION_ID", "S")
        .envs(vars.iter().copied());
    let script = resources().join("ngterm-agent-status.sh");
    run_under(&stub, command, chain, &script, &["active", "--blink"], "");
    fs::read_to_string(&call).ok()
}

/// Pi and `OpenCode` report through the shared wrapper alone, and a headless run of either (`pi -p`, `opencode run`)
/// that another agent started inside a pane inherits the pane's `NGTERM_*`: the wrapper's own ancestor walk keeps it
/// silent, with the Claude adapter's rule and pane boundary. An adapter that has already walked says so.
#[cfg(unix)]
#[test]
fn the_wrapper_reports_nothing_for_an_agent_under_another_agent_in_the_pane() {
    const CALL: &str = "session status active --target S --blink";
    for agent in ["pi", "opencode"] {
        let alone = wrapper_under(&format!("ngterm-session shell {agent}"), &[]);
        assert_eq!(alone.as_deref(), Some(CALL), "{agent}: the pane's agent");
        let worker = wrapper_under(&format!("ngterm-session shell claude shell {agent}"), &[]);
        assert_eq!(worker, None, "{agent}: a worker under another agent");
        let above = wrapper_under(&format!("claude shell ngterm-headless shell {agent}"), &[]);
        assert_eq!(
            above.as_deref(),
            Some(CALL),
            "{agent}: an agent above the pane's origin"
        );
    }
    let walked = wrapper_under("ngterm shell claude shell pi", &[("NGTERM_STATUS_WALKED", "1")]);
    assert_eq!(walked.as_deref(), Some(CALL), "the caller's walk stands");
    assert_eq!(
        wrapper_under("ngterm shell", &[]).as_deref(),
        Some(CALL),
        "a shell's own call"
    );
}

#[test]
fn a_stop_with_background_work_in_flight_stays_active() {
    let stop = payload("Stop", "started the tests", IN_FLIGHT);
    assert_eq!(forwarded(&["completed", "--auto-reset"], &stop), "active --blink");
    let spaced = stop
        .replace(r#""hook_event_name":"#, r#""hook_event_name": "#)
        .replace(r#""background_tasks":["#, "\"background_tasks\": [\n  ");
    assert_eq!(forwarded(&["completed", "--auto-reset"], &spaced), "active --blink");
}

#[test]
fn a_stop_with_nothing_in_flight_completes() {
    let done = payload("Stop", "all green", "[]");
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &done),
        "completed --auto-reset"
    );
    // a key quoted in the last message is escaped, so it is not the payload's list
    let quoted = payload("Stop", r#"saw \"background_tasks\":[{\"id\":1}] in the log"#, "[]");
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &quoted),
        "completed --auto-reset"
    );
    // an older Claude Code sends no list at all
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], r#"{"hook_event_name":"Stop"}"#),
        "completed --auto-reset"
    );
}

#[test]
fn other_states_and_events_forward_verbatim() {
    let stop = payload("Stop", "started the tests", IN_FLIGHT);
    assert_eq!(forwarded(&["active", "--blink"], &stop), "active --blink");
    assert_eq!(forwarded(&["blocked"], &stop), "blocked");
    let subagent = payload("SubagentStop", "started the tests", IN_FLIGHT);
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &subagent),
        "completed --auto-reset"
    );
    assert_eq!(forwarded(&["completed", "--auto-reset"], ""), "completed --auto-reset");
}

/// The ancestor walk on Windows, through the process table: copies of `cmd.exe` named as agents stand between a
/// launcher and the adapter, each running the next with `/c`. One agent, or a run of one name, is the pane's agent and
/// reports; a second, different agent above the first makes the hook a worker's, which stays silent, unless the
/// pane's origin (the app, a headless origin, the session server) stands between the two. The launcher
/// starts the first agent with `start /b` and exits, so the walk ends there, never reaching an agent that may be
/// running these tests; the outermost agent touches a `done` file once the chain has finished. Each chain records to
/// files of its own: a file a finished chain's agent still holds open cannot be deleted for the next one (its sharing
/// refuses it), so a shared `done` could stand from the last chain before this one ran. The paths go unquoted, since
/// `cmd /c` strips the quotes of a command line that starts with one: the test's directories hold no space.
#[cfg(windows)]
#[test]
fn a_hook_under_a_second_agent_is_a_workers_and_stays_silent() {
    use std::os::windows::process::CommandExt as _;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    for agent in ["claude", "codex", "omp", "ngterm", "ngterm-headless", "ngterm-session"] {
        let copy = dir.path().join(format!("{agent}.exe"));
        fs::copy(Path::new(&system).join(r"System32\cmd.exe"), copy).unwrap();
    }
    let stub = dir.path().join("wrapper.ps1");
    fs::write(
        &stub,
        "Set-Content -LiteralPath $env:OUT -Value ($args -join ' ') -NoNewline\r\n",
    )
    .unwrap();
    let adapter = resources().join("ngterm-claude-status.ps1");
    for path in [dir.path(), adapter.as_path()] {
        assert!(
            !path.display().to_string().contains(' '),
            "a space in {}",
            path.display()
        );
    }
    let hook = format!(
        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {} blocked",
        adapter.display()
    );
    let exe = |name: &str| dir.path().join(format!("{name}.exe")).display().to_string();
    let under = |agents: &[&str]| {
        let chain = agents.join("-");
        let (out, done) = (
            dir.path().join(format!("{chain}.argv")),
            dir.path().join(format!("{chain}.done")),
        );
        let line = agents
            .iter()
            .rev()
            .fold(hook.clone(), |inner, name| format!("{} /d /c {inner}", exe(name)));
        // `^&` and `^>` reach the first agent, which runs the chain and then touches `done`
        let status = Command::new("cmd")
            .raw_arg(format!("/d /c start \"\" /b {line} ^& type nul ^> {}", done.display()))
            .env("OUT", &out)
            .env("NGTERM_STATUS_WRAPPER", &stub)
            .env("NGTERM_SESSION_ID", "S")
            .env_remove("NGTERM_AGENT_RE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .status()
            .unwrap();
        assert!(status.success(), "{agents:?}: {status}");
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done.exists() {
            assert!(Instant::now() < deadline, "{agents:?}: the chain never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
        fs::read_to_string(&out).ok()
    };
    assert_eq!(under(&["claude"]).as_deref(), Some("blocked"), "the pane's agent");
    assert_eq!(
        under(&["claude", "claude"]).as_deref(),
        Some("blocked"),
        "a run of one name"
    );
    assert_eq!(under(&["claude", "codex"]), None, "a worker under another agent");
    // the walk ends at the pane's origin: an agent above it started the origin and is not in the pane
    for origin in ["ngterm", "ngterm-headless", "ngterm-session"] {
        assert_eq!(
            under(&["codex", origin, "claude"]).as_deref(),
            Some("blocked"),
            "{origin}: the pane's agent under an agent above its origin"
        );
        assert_eq!(
            under(&[origin, "codex", "claude"]),
            None,
            "{origin}: a worker under another agent in the pane"
        );
    }
    assert_eq!(
        under(&["omp", "claude"]),
        None,
        "a Claude worker must not reset the OMP pane"
    );
}
