//! The Codex status adapter (`resources/agent-status/ngterm-codex-status.sh`) on every system: Codex reads a hook's
//! output to its end before it goes on, so the events that start the screen watcher must hand that output back when
//! the adapter exits, while the watcher keeps going. Under Git Bash on Windows a Codex session was seen waiting at
//! "Running hook" while the watcher lived, so there the adapter starts no watcher
//! (`build/patches/agent-status/0011-codex-windows-watcher.patch`); elsewhere it starts one as before. The adapter runs for real, with recording stand-ins for the status wrapper
//! (`NGTERM_STATUS_WRAPPER`) and `ngtermctl` (`NGTERMCTL`), and no `NGTERM_*` of this process reaches it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ngterm_host::fakes::write_executable;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Longer than a hook takes, far shorter than the watcher's run.
const HOOK_DEADLINE: Duration = Duration::from_secs(30);

/// Git for Windows' bash, where its installer puts it (Codex's hook command names it there); `bash` elsewhere.
fn bash() -> Option<PathBuf> {
    if cfg!(windows) {
        ["ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|dir| PathBuf::from(dir).join("Git").join("bin").join("bash.exe"))
            .find(|bash| bash.is_file())
    } else {
        Some(PathBuf::from("bash"))
    }
}

/// `path` as bash takes it on every system: forward slashes.
fn posix(path: &Path) -> TestResult<String> {
    Ok(path.to_str().ok_or("a UTF-8 path")?.replace('\\', "/"))
}

fn hook() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/ngterm-codex-status.sh")
}

/// What one adapter run left behind.
struct HookRun {
    output: Output,
    statuses: String,
    controls: String,
    watcher_token: Option<String>,
    /// Holds the token file: dropping it ends a watcher at its next check.
    _dir: tempfile::TempDir,
}

/// Runs the adapter for `action` as Codex does, its output piped and read to the end; `None` after printing why when
/// this machine has no bash.
fn run(action: &str) -> TestResult<Option<HookRun>> {
    let Some(bash) = bash() else {
        eprintln!("SKIPPED: {action}: no Git for Windows bash");
        return Ok(None);
    };
    let dir = tempfile::tempdir()?;
    let statuses = dir.path().join("statuses");
    let controls = dir.path().join("controls");
    let token_file = dir.path().join("watch-token");
    let status_wrapper = dir.path().join("status-wrapper");
    write_executable(
        &status_wrapper,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", posix(&statuses)?),
    )?;
    let ngtermctl = dir.path().join("ngtermctl");
    write_executable(
        &ngtermctl,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n\
             [ \"$1 $2\" = 'agent mail' ] && printf '%s\\n' '{{\"additionalContext\":\"mail\"}}'\nexit 0\n",
            posix(&controls)?
        ),
    )?;

    let mut command = Command::new(bash);
    command.arg(posix(&hook())?).arg(action);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let mut child = command
        .env("NGTERMCTL", posix(&ngtermctl)?)
        .env("NGTERM_STATUS_WRAPPER", posix(&status_wrapper)?)
        .env("NGTERM_SESSION_ID", "sid")
        .env("NGTERM_SOCKET", posix(&dir.path().join("ngterm.sock"))?)
        .env("NGTERM_PANE_ID", "stable-token")
        .env("NGTERM_CODEX_WATCH_FILE", posix(&token_file)?)
        .env("NGTERM_CODEX_WATCH_MAX_CHECKS", "600")
        .env("NGTERM_CODEX_WATCH_INTERVAL", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        stdin.write_all(br#"{"hook_event_name":"x","transcript_path":"rollout.jsonl"}"#)?;
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || sender.send(child.wait_with_output()));
    let output = receiver.recv_timeout(HOOK_DEADLINE).map_err(|_| {
        format!(
            "{action}: the hook's output did not end within {HOOK_DEADLINE:?}: Codex would wait at \"Running hook\""
        )
    })??;
    Ok(Some(HookRun {
        output,
        statuses: fs::read_to_string(&statuses).unwrap_or_default(),
        controls: fs::read_to_string(&controls).unwrap_or_default(),
        watcher_token: fs::read_to_string(&token_file).ok(),
        _dir: dir,
    }))
}

/// The watcher runs everywhere but under Git Bash.
fn assert_watcher(run: &HookRun, action: &str) {
    if cfg!(windows) {
        assert_eq!(run.watcher_token, None, "{action}: a watcher under Git Bash");
    } else {
        assert!(run.watcher_token.is_some(), "{action}: no watcher");
    }
}

#[test]
fn a_prompt_hands_its_output_back_at_once_with_status_and_mail() -> TestResult {
    let Some(run) = run("user-prompt-submit")? else {
        return Ok(());
    };
    let stderr = String::from_utf8_lossy(&run.output.stderr);
    assert_eq!(run.output.status.code(), Some(0), "{stderr}");
    assert_eq!(run.statuses.lines().collect::<Vec<_>>(), ["active --blink"]);
    assert!(
        String::from_utf8_lossy(&run.output.stdout).contains(r#"{"additionalContext":"mail"}"#),
        "{stderr}"
    );
    assert!(
        run.controls.contains("agent hook codex UserPromptSubmit"),
        "{}",
        run.controls
    );
    assert_watcher(&run, "user-prompt-submit");
    Ok(())
}

#[test]
fn a_permission_request_hands_its_output_back_at_once_with_the_agent_record() -> TestResult {
    let Some(run) = run("permission-request")? else {
        return Ok(());
    };
    assert_eq!(
        run.output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert!(
        run.controls.contains("agent hook codex PermissionRequest"),
        "{}",
        run.controls
    );
    assert_watcher(&run, "permission-request");
    Ok(())
}
