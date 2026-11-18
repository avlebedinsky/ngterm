//! The Windows child and its job on real pseudoconsoles: a handed-over child killed through its process handle, a
//! retired killer, handles without the rights a call needs, a job holding more processes than a first read of its
//! list has room for, the spawn's and the master's refusals, the suspended start a denied one falls back to and its
//! failure, and the processes that left a job taken back into it, but not a process that got a listed one's id. An
//! actor reads each child's output away, so the console never waits on its pipe.

use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use portable_pty::ChildKiller;
use windows_sys::Win32::Foundation::{DuplicateHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

use super::*;
use crate::{Actor, ActorConfig, ActorHandle, Command};

/// How long a test waits for a child (a CI runner can be slow to start one).
const PATIENCE: Duration = Duration::from_secs(60);
/// The job access right that reads its information (`JOB_OBJECT_QUERY`, winnt.h).
const JOB_QUERY: u32 = 0x0004;
/// The job access right that terminates its processes (`JOB_OBJECT_TERMINATE`, winnt.h).
const JOB_TERMINATE: u32 = 0x0008;

/// `program` with `args`, without the live ngterm's variables.
fn command(program: &str, args: &[&str]) -> Command {
    std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            key.to_str()
                .is_some_and(|key| key.starts_with("NGTERM_") || key.starts_with("AGTERM_"))
        })
        .fold(Command::new(program).args(args), Command::env_remove)
}

/// An actor that reads the console's output away, answering the cursor query it starts with.
fn drain(master: Master) -> ActorHandle {
    let on_read = Box::new(|bytes: &[u8]| {
        if bytes.windows(4).any(|window| window == b"\x1b[6n") {
            vec![b"\x1b[1;1R".to_vec()]
        } else {
            Vec::new()
        }
    });
    Actor::spawn(ActorConfig::new("test", master, on_read)).expect("an actor")
}

/// A child on a pseudoconsole and the actor reading its output; dropped, its job's close ends its tree.
struct Running {
    child: Child,
    _actor: ActorHandle,
}

fn run(program: &str, args: &[&str]) -> Running {
    let Spawned { master, child } =
        crate::spawn(&command(program, args), Size::cells(24, 80)).expect("a child on a pseudoconsole");
    Running {
        child,
        _actor: drain(master),
    }
}

/// A child that runs until it is killed.
fn lasting() -> Running {
    run("cmd.exe", &["/d", "/c", "ping -n 300 127.0.0.1 >nul"])
}

/// A copy of `handle` with only the `access` rights.
fn restricted(handle: BorrowedHandle<'_>, access: u32) -> OwnedHandle {
    // SAFETY: returns this process's pseudo-handle; no memory is passed.
    let this = unsafe { GetCurrentProcess() };
    let mut copy = std::ptr::null_mut();
    // SAFETY: duplicates a handle this process holds open into this process, written to a live local.
    let copied = unsafe { DuplicateHandle(this, handle.as_raw_handle(), this, &raw mut copy, access, 0, 0) };
    assert_ne!(copied, 0, "{}", io::Error::last_os_error());
    // SAFETY: a fresh handle nothing else owns.
    unsafe { OwnedHandle::from_raw_handle(copy) }
}

/// `child` as another process takes it over: copies of its process handle and its job with every right, or with
/// only the rights given.
fn handed_over(child: &Child, process_access: Option<u32>, job_access: Option<u32>) -> Child {
    let process = child.process().expect("the child's process handle");
    let process = match process_access {
        Some(access) => restricted(process, access),
        None => process.try_clone_to_owned().expect("a copy of the process handle"),
    };
    let job = match job_access {
        Some(access) => restricted(child.job(), access),
        None => child.job().try_clone_to_owned().expect("a copy of the job"),
    };
    Child::from_handoff(process, job).expect("the handed-over child")
}

fn os_error(err: &io::Error) -> Option<u32> {
    err.raw_os_error().and_then(|code| u32::try_from(code).ok())
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_masters_debug_shows_its_pipes() {
    let Spawned { master, mut child } = crate::spawn(&command("cmd.exe", &["/d", "/c", "exit 0"]), Size::cells(24, 80))
        .expect("a child on a pseudoconsole");
    let shown = format!("{master:?}");
    assert!(
        shown.starts_with("Master { input: ") && shown.contains(", output: ") && shown.ends_with(", .. }"),
        "{shown}"
    );
    let actor = drain(master);
    assert!(child.wait().expect("the child's exit").success());
    actor.child_exited();
}

/// A handed-over child is killed through its process handle (exit code 1), and its id read through it; killing it
/// again, gone, fails.
#[test]
fn a_handed_over_child_is_killed_through_its_process_handle() {
    let running = lasting();
    let mut handed = handed_over(&running.child, None, None);
    assert_eq!(handed.inner.process_id(), Some(running.child.pid()));
    handed.inner.kill().expect("the kill");
    assert_eq!(handed.wait().expect("the exit").exit_code(), 1);
    assert_eq!(
        handed.try_wait().expect("the status").map(|status| status.exit_code()),
        Some(1)
    );
    let again = handed.inner.kill().expect_err("killed twice");
    assert_eq!(os_error(&again), Some(ERROR_ACCESS_DENIED), "{again}");
}

/// A process killed while it is still creating its own child takes that child with it (`STATUS_PROCESS_IS_TERMINATING`,
/// with or without a job or a console), so the kill waits for a grandchild that has run: a job listing the inner
/// cmd's ping shows the inner cmd got far enough to start it.
#[test]
fn a_handed_over_childs_killer_clone_kills_it() {
    let running = run("cmd.exe", &["/d", "/c", "cmd /d /c ping -n 300 127.0.0.1 >nul"]);
    let mut handed = handed_over(&running.child, None, None);
    let view = running.child.job_view();
    wait_until("the inner cmd started its ping", || {
        view.process_ids().is_some_and(|pids| pids.len() >= 3)
    });
    let mut killer = handed.inner.clone_killer();
    killer.kill().expect("the kill");
    assert_eq!(handed.wait().expect("the exit").exit_code(), 1);
    wait_until("the child left its job", || {
        view.process_ids()
            .is_some_and(|pids| !pids.contains(&running.child.pid()))
    });
    assert!(
        !view.process_ids().expect("the job's processes").is_empty(),
        "the child's kill ended its tree"
    );
}

/// A handed-over child hands its process handle on again: the next owner's copy names the same process.
#[test]
fn a_handed_over_child_hands_its_process_handle_on() {
    let running = lasting();
    let handed = handed_over(&running.child, None, None);
    let again = handed_over(&handed, None, None);
    assert_eq!(again.pid(), running.child.pid());
}

#[test]
fn a_killer_without_a_handle_kills_nothing() {
    let refused = "no handle to kill the handed-over child through";
    let mut killer = NoKiller;
    assert_eq!(killer.kill().expect_err("killed").to_string(), refused);
    assert_eq!(killer.clone_killer().kill().expect_err("killed").to_string(), refused);
}

#[test]
fn a_handle_that_names_no_process_is_refused() {
    let job = kill_on_close_job().expect("a job");
    let not_a_process = job.try_clone().expect("a copy of the job");
    let err = Child::from_handoff(not_a_process, job).expect_err("a job taken for a process");
    assert_eq!(os_error(&err), Some(ERROR_INVALID_HANDLE), "{err}");
}

#[test]
fn a_process_handle_without_the_right_to_wait_fails_the_wait() {
    let running = lasting();
    let mut handed = handed_over(&running.child, Some(PROCESS_QUERY_LIMITED_INFORMATION), None);
    assert_eq!(handed.pid(), running.child.pid());
    let err = handed.try_wait().expect_err("polled without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    let err = handed.wait().expect_err("waited without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
}

/// A job handle that may only be queried lists the tree but cannot end it; one that may only terminate ends it but
/// lists nothing.
#[test]
fn a_job_handle_without_a_right_fails_what_needs_it() {
    let mut running = lasting();
    let pid = running.child.pid();
    let query_only = handed_over(&running.child, None, Some(JOB_QUERY));
    let err = query_only
        .killer()
        .kill_tree()
        .expect_err("terminated without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    let pids = query_only.job_view().process_ids().expect("the job's processes");
    assert!(pids.contains(&pid), "{pids:?}");
    assert!(
        running.child.try_wait().expect("the status").is_none(),
        "the child ended"
    );

    let terminate_only = handed_over(&running.child, None, Some(JOB_TERMINATE));
    assert_eq!(terminate_only.job_view().process_ids(), None);
    terminate_only.killer().hang_up().expect("the hangup");
    assert_eq!(running.child.wait().expect("the exit").exit_code(), 1);
}

/// The job, not a process id, names the tree, so a retired killer still ends it.
#[test]
fn a_retired_killer_still_ends_the_tree() {
    let mut running = lasting();
    let killer = running.child.killer();
    killer.retire();
    killer.kill_tree().expect("the kill");
    running.child.wait_exit().expect("the exit");
    assert_eq!(
        running
            .child
            .try_wait()
            .expect("the status")
            .map(|status| status.exit_code()),
        Some(1)
    );
}

#[test]
fn a_job_view_lists_more_processes_than_a_first_read_has_room_for() {
    let children = FIRST_CAPACITY + 8;
    let line =
        format!("(for /l %i in (1,1,{children}) do @start /b ping -n 300 127.0.0.1 >nul) & ping -n 300 127.0.0.1 >nul");
    let running = run("cmd.exe", &["/d", "/c", &line]);
    let view = running.child.job_view();
    // cmd, the pings it starts and the one it waits for
    let all = children + 2;
    wait_until("the job holds every ping", || {
        view.process_ids().is_some_and(|pids| pids.len() >= all)
    });
    let mut pids = view.process_ids().expect("the job's processes");
    assert!(pids.contains(&running.child.pid()), "{pids:?}");
    pids.sort_unstable();
    pids.dedup();
    assert!(pids.len() >= all, "listed twice: {pids:?}");
}

/// A pseudoconsole of no size is refused with portable-pty's error (`E_INVALIDARG`).
#[test]
fn a_console_of_no_size_is_refused() {
    let no_size = crate::spawn(&command("cmd.exe", &["/d", "/c", "exit 0"]), Size::cells(0, 0));
    let err = no_size.expect_err("a pseudoconsole of no size");
    assert_eq!(err.to_string(), "failed to create psuedo console: HRESULT -2147024809");
}

/// A program that is not there is refused in plain words: the program as written, then the OS error (no debug text:
/// doubled backslashes, NUL terminators, `Some(...)`).
#[test]
fn a_missing_program_is_refused_naming_it_as_written() {
    let program = r"C:\ngterm-no-such-dir\no-such-program.exe";
    let err = crate::spawn(&command(program, &["-l"]), Size::cells(24, 80)).expect_err("a program that is not there");
    let shown = err.to_string();
    assert_eq!(err.kind(), io::ErrorKind::NotFound, "{shown}");
    assert!(
        shown.starts_with(&format!("{program}: ")) && shown.ends_with("(os error 3)") && shown.lines().count() == 1,
        "{shown}"
    );
    for debug_text in [r"\\", r"\0", "Some(", "CreateProcessW"] {
        assert!(!shown.contains(debug_text), "{debug_text} in {shown}");
    }
}

/// A console whose pipes were taken already makes no master.
#[test]
fn a_console_without_its_pipes_makes_no_master() {
    let pair = ConPtySystem::default()
        .openpty(Size::cells(24, 80).pty())
        .expect("a pseudoconsole");
    drop(pair.slave);
    let master: Box<dyn MasterPty> = pair.master;
    let conpty = master
        .downcast::<ConPtyMasterPty>()
        .map_err(|_| "another kind of master")
        .expect("a ConPTY master");
    drop(conpty.take_pipes().expect("the pipes"));
    let err = Master::new(conpty).expect_err("a master without pipes");
    assert_eq!(err.to_string(), "writer already taken");
}

/// Dimensions past the signed COORD range are refused without wrapping the requested size.
#[test]
fn an_oversized_console_is_refused_with_the_requested_size() {
    for (rows, cols) in [(24, 32_768), (32_768, 80), (40_000, 40_000), (u16::MAX, u16::MAX)] {
        let err = ConPtySystem::default()
            .openpty(Size::cells(rows, cols).pty())
            .err()
            .expect("a pseudoconsole past i16");
        assert_eq!(
            err.to_string(),
            format!("pseudoconsole size {cols}x{rows} exceeds the maximum 32767x32767")
        );
    }
}

/// A spawn of `program` with `args` through [`spawn_with`], each start refused as `refuse` says or else made, and
/// the starts it asked for, in order.
fn spawn_recording(
    program: &str,
    args: &[&str],
    refuse: impl Fn(Joining) -> Option<Refusal>,
) -> (io::Result<Spawned>, Vec<Joining>) {
    let (spawned, made) = spawn_recording_jobs(program, args, refuse);
    (spawned, made.into_iter().map(|(joining, _)| joining).collect())
}

/// [`spawn_recording`], with the job each start was given (its handle's value).
fn spawn_recording_jobs(
    program: &str,
    args: &[&str],
    refuse: impl Fn(Joining) -> Option<Refusal>,
) -> (io::Result<Spawned>, Vec<(Joining, usize)>) {
    let made = std::sync::Mutex::new(Vec::new());
    let spawned = spawn_with(
        command(program, args).builder(),
        Size::cells(24, 80),
        |builder, size, job, joining| {
            made.lock()
                .expect("the starts")
                .push((joining, job.as_raw_handle().addr()));
            refuse(joining).map_or_else(|| start(builder, size, job, joining), Err)
        },
    );
    (spawned, made.into_inner().expect("the starts"))
}

/// The refusal of a start in a job that Windows denies (`ERROR_ACCESS_DENIED`), for the start in the job only.
fn denied_in_job(joining: Joining) -> Option<Refusal> {
    (joining == Joining::AtCreation).then(|| Refusal {
        error: io::Error::from_raw_os_error(i32::try_from(ERROR_ACCESS_DENIED).expect("an error code")),
        denied: true,
    })
}

/// The exit code of the process `process` names, once it exits; `None` if it still runs `PATIENCE` from now.
fn exit_code_of(process: &OwnedHandle) -> Option<u32> {
    let patience = u32::try_from(PATIENCE.as_millis()).expect("a timeout");
    // SAFETY: waits on a process handle this test holds open with `SYNCHRONIZE`.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), patience) } != WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0_u32;
    // SAFETY: reads the exit code of a process this test holds open into a live local.
    (unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } != 0).then_some(code)
}

/// Process `pid` with the rights to wait for it and read its state; `None` once it is gone.
fn watched(pid: u32) -> Option<OwnedHandle> {
    // SAFETY: opens a process by id; no memory is passed.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    // SAFETY: a fresh handle nothing else owns.
    (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// The process running `program` (a file name) that the system lists as started by `pid`, once there is one (a
/// console program's first child can be its console host).
fn child_of(pid: u32, program: &str) -> u32 {
    let mut found = None;
    wait_until("the process started its child", || {
        found = escapees::processes()
            .iter()
            .filter(|entry| entry.parent == pid)
            .find(|entry| watched(entry.pid).is_some_and(|process| image_name(&process).eq_ignore_ascii_case(program)))
            .map(|entry| entry.pid);
        found.is_some()
    });
    found.expect("a child")
}

/// A child whose start in its job Windows denies (a packaged app's, PowerShell from the Microsoft Store while another
/// of its processes runs) starts suspended on a new pseudoconsole, in a new job (the denied start's refuses the app's
/// processes), instead: it joins the job before it runs, so what it starts is in the job too, and the job's end ends
/// them.
#[test]
fn a_denied_start_starts_the_child_suspended_in_a_new_job() {
    let (spawned, made) = spawn_recording_jobs(
        "cmd.exe",
        &["/d", "/c", "cmd /d /c ping -n 300 127.0.0.1 >nul"],
        denied_in_job,
    );
    let [(Joining::AtCreation, denied_job), (Joining::Suspended, job)] = made[..] else {
        panic!("the starts: {made:?}");
    };
    assert_ne!(denied_job, job, "the suspended start got the denied start's job");
    let Spawned { master, mut child } = spawned.expect("the suspended start");
    assert_eq!(child.job().as_raw_handle().addr(), job);
    let _actor = drain(master);
    assert!(!escapees::packaged(child.process().expect("the process handle")));
    let view = child.job_view();
    wait_until("the inner cmd started its ping in the job", || {
        view.process_ids().is_some_and(|pids| pids.len() >= 3)
    });
    assert!(view.process_ids().expect("the job's processes").contains(&child.pid()));
    child.killer().kill_tree().expect("the kill");
    assert_eq!(child.wait().expect("the exit").exit_code(), 1);
    wait_until("the tree ended", || {
        view.process_ids().is_some_and(|pids| pids.is_empty())
    });
}

/// A child started suspended runs on its own pseudoconsole: what it prints reaches the master. The line is not
/// the one the command names, which the console's title would carry too.
#[test]
fn a_child_started_suspended_prints_to_its_console() {
    let (spawned, _) = spawn_recording("cmd.exe", &["/d", "/c", "echo ngterm-%OS%"], denied_in_job);
    let Spawned { master, mut child } = spawned.expect("the suspended start");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let on_read = {
        let seen = Arc::clone(&seen);
        Box::new(move |bytes: &[u8]| {
            seen.lock().expect("the output").extend_from_slice(bytes);
            if bytes.windows(4).any(|window| window == b"\x1b[6n") {
                vec![b"\x1b[1;1R".to_vec()]
            } else {
                Vec::new()
            }
        })
    };
    let actor = Actor::spawn(ActorConfig::new("test", master, on_read)).expect("an actor");
    assert!(child.wait().expect("the exit").success());
    wait_until("the console printed the line", || {
        String::from_utf8_lossy(&seen.lock().expect("the output")).contains("ngterm-Windows_NT")
    });
    actor.child_exited();
}

/// Only a denied start is made again: another refusal is the spawn's error, after one start.
#[test]
fn a_start_refused_otherwise_is_not_made_again() {
    let (spawned, made) = spawn_recording("cmd.exe", &["/d", "/c", "exit 0"], |_| {
        Some(Refusal::other(io::Error::other("no console for the child")))
    });
    assert_eq!(made, [Joining::AtCreation]);
    let err = spawned.expect_err("a refused start");
    assert_eq!(err.to_string(), "no console for the child");
}

/// A program Windows denies however it starts (a directory) is refused after both starts, in plain words.
#[test]
fn a_program_denied_both_starts_is_refused_naming_it() {
    let program = std::env::var("SystemRoot").expect("the Windows directory");
    let (spawned, made) = spawn_recording(&program, &[], |_| None);
    assert_eq!(made, [Joining::AtCreation, Joining::Suspended]);
    let err = spawned.expect_err("a directory started");
    let shown = err.to_string();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{shown}");
    assert!(
        shown.starts_with(&format!("{program}: ")) && shown.ends_with("(os error 5)"),
        "{shown}"
    );
}

/// What a packaged shell leaves, made without a package: `line` run by a `cmd.exe` outside any job of the test's
/// own, and that `cmd.exe` then put alone into a new kill-on-close job once it started `program`, so what it started
/// is outside the job.
struct Escaped {
    job: Arc<Job>,
    shell: std::process::Child,
}

impl Escaped {
    fn new(line: &str, program: &str) -> Self {
        let shell = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", line])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("a shell");
        // what the shell starts is in no job of the test's yet
        child_of(shell.id(), program);
        let job = kill_on_close_job().expect("a job");
        // SAFETY: both handles are open, the process one in `shell`.
        let joined = unsafe { AssignProcessToJobObject(job.as_raw_handle(), shell.as_raw_handle()) };
        assert_ne!(joined, 0, "{}", io::Error::last_os_error());
        Self {
            job: Job::holding(job, false),
            shell,
        }
    }

    fn killer(&self) -> Killer {
        Killer {
            job: Arc::clone(&self.job),
        }
    }

    fn holds(&self, pid: u32) -> bool {
        job_process_ids(&self.job.handle).is_some_and(|pids| pids.contains(&pid))
    }
}

impl Drop for Escaped {
    fn drop(&mut self) {
        drop(self.shell.kill());
    }
}

/// The kill of a tree takes back first what escaped its job, and ends it too: only what its processes started.
#[test]
fn killing_the_tree_ends_what_escaped_its_job() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    assert!(!escaped.holds(ping), "the ping started in the job");
    let ping = watched(ping).expect("the ping");
    let mut bystander = std::process::Command::new("ping")
        .args(["-n", "300", "127.0.0.1"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("a process outside the tree");
    escaped.killer().kill_tree().expect("the kill");
    assert_eq!(exit_code_of(&ping), Some(1));
    assert!(
        bystander.try_wait().expect("the bystander's state").is_none(),
        "the kill ended another tree"
    );
    bystander.kill().expect("the bystander's end");
}

/// The close of a job takes back what escaped it first, so the close ends it with the tree.
#[test]
fn the_close_of_a_job_ends_what_escaped_it() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = watched(child_of(escaped.shell.id(), "ping.exe")).expect("the ping");
    drop(escaped);
    // a kill on close ends a process with the exit code 0
    assert_eq!(exit_code_of(&ping), Some(0), "the ping outlived its job");
}

/// A watched job (a packaged program's) takes back what escaped it while the tree runs: the job lists it then.
#[test]
fn a_watched_job_takes_back_what_escaped_it() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    escapees::watch(&escaped.job);
    wait_until("the job took the ping back", || escaped.holds(ping));
}

/// What an escaped process started before it was taken back escaped too, and is taken back with it.
#[test]
fn what_an_escaped_process_started_is_taken_back_too() {
    let escaped = Escaped::new("cmd /d /c ping -n 300 127.0.0.1 >nul", "cmd.exe");
    let inner = child_of(escaped.shell.id(), "cmd.exe");
    let ping = watched(child_of(inner, "ping.exe")).expect("the ping");
    escaped.killer().kill_tree().expect("the kill");
    assert_eq!(exit_code_of(&ping), Some(1));
}

/// A process table read before a process ended can name its id as a child of the tree, after another process got
/// that id: that process is left alone, as its creator, read through its own handle, is not of the tree. It is
/// created after the tree's process, so the creation time alone does not tell.
#[test]
fn a_process_that_got_the_id_of_a_listed_child_is_not_taken() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    let mut bystander = std::process::Command::new("ping")
        .args(["-n", "300", "127.0.0.1"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("a process outside the tree");
    let mut listed = escapees::processes();
    listed.push(escapees::Entry {
        pid: bystander.id(),
        parent: escaped.shell.id(),
    });
    escapees::Tree::of(&escaped.job.handle).adopt(&escaped.job.handle, &listed);
    assert!(escaped.holds(ping), "the shell's own ping stayed out");
    assert!(!escaped.holds(bystander.id()), "a process of another tree was taken");
    bystander.kill().expect("the bystander's end");
    bystander.wait().expect("the bystander's exit");
}

/// What a process of the tree starts outside the job as the kill ends it is taken back once the tree ended, and
/// ended too: the tree holds that process open, so its id still names it.
#[test]
fn what_a_process_started_as_its_tree_was_killed_is_taken_back() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = watched(child_of(escaped.shell.id(), "ping.exe")).expect("the ping");
    let shell = watched(escaped.shell.id()).expect("the shell");
    let job = &escaped.job.handle;
    // the ping, outside the job and not yet taken back, stands for one started after the first pass
    let mut tree = escapees::Tree::of(job);
    terminate(job).expect("the kill");
    tree.wait_ended(escapees::ENDING);
    assert_eq!(exit_code_of(&shell), Some(1));
    tree.adopt(job, &escapees::processes());
    terminate(job).expect("the second kill");
    assert_eq!(exit_code_of(&ping), Some(1), "the ping outlived its tree");
}

/// A child started suspended that cannot join its job (a job handle without the right to assign) is terminated
/// before it runs, and the start fails with the assignment's error.
#[test]
fn a_suspended_child_that_cannot_join_its_job_is_terminated() {
    let pair = ConPtySystem::default()
        .openpty(Size::cells(24, 80).pty())
        .expect("a pseudoconsole");
    drop(pair.slave);
    let master: Box<dyn MasterPty> = pair.master;
    let Ok(conpty) = master.downcast::<ConPtyMasterPty>() else {
        panic!("the master is not a ConPTY master");
    };
    let (child, thread) = conpty
        .spawn_command_suspended(command("cmd.exe", &["/d", "/c", "exit 0"]).builder())
        .expect("a suspended child");
    let process = watched(child.process_id().expect("the child's id")).expect("the child");
    let job = kill_on_close_job().expect("a job");
    let query_only = restricted(job.as_handle(), JOB_QUERY);
    let err = join_and_resume(child, &thread, &query_only).expect_err("joined without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    assert_eq!(
        exit_code_of(&process),
        Some(1),
        "the suspended child was not terminated"
    );
}

/// PowerShell from the Microsoft Store (a packaged app) as a pane's program, the issue's setup: the first pane starts
/// in its job, the second, while the first runs, only suspended, as Windows then denies the start in a job. What
/// either starts leaves its job and is taken back: a ping on the console and a detached one end with the pane, by
/// its kill and by its job's close. Needs that PowerShell (`Microsoft.PowerShell` from the Store or its
/// msixbundle), which CI does not have.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn a_store_powershell_pane_starts_and_its_tree_ends_with_it() {
    let local = std::env::var("LOCALAPPDATA").expect("the local app data folder");
    let alias = format!(r"{local}\Microsoft\WindowsApps\pwsh.exe");
    let line = "Start-Process -WindowStyle Hidden ping -ArgumentList '-n','300','127.0.0.1'; ping -n 300 127.0.0.1";
    let args = ["-NoLogo", "-NoProfile", "-Command", line];
    let (first, made) = spawn_recording(&alias, &args, |_| None);
    assert_eq!(made, [Joining::AtCreation], "another Store PowerShell runs");
    let (second, made) = spawn_recording(&alias, &args, |_| None);
    assert_eq!(made, [Joining::AtCreation, Joining::Suspended]);
    let mut pings = Vec::new();
    let mut panes = Vec::new();
    for Spawned { master, child } in [first.expect("the first pane"), second.expect("the second pane")] {
        assert!(escapees::packaged(child.process().expect("the process handle")));
        let actor = drain(master);
        pings.push(pings_of(&child));
        panes.push((child, actor));
    }
    let (second, _actor) = panes.pop().expect("the second pane");
    second.killer().kill_tree().expect("the kill");
    for ping in pings.pop().expect("the second pane's pings") {
        assert_eq!(exit_code_of(&ping), Some(1), "a ping outlived the second pane's kill");
    }
    drop(panes);
    for ping in pings.pop().expect("the first pane's pings") {
        assert_eq!(exit_code_of(&ping), Some(0), "a ping outlived the first pane's close");
    }
}

/// The two pings a Store PowerShell pane's line starts, once its job took both back.
fn pings_of(child: &Child) -> Vec<OwnedHandle> {
    let view = child.job_view();
    let mut pings = Vec::new();
    wait_until("the job took both pings back", || {
        pings = view
            .process_ids()
            .unwrap_or_default()
            .into_iter()
            .filter_map(watched)
            .filter(|process| image_name(process).eq_ignore_ascii_case("ping.exe"))
            .collect();
        pings.len() == 2
    });
    pings
}

/// The file name of the program `process` runs.
fn image_name(process: &OwnedHandle) -> String {
    use windows_sys::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
    let mut path = [0_u16; 1024];
    let mut length = u32::try_from(path.len()).expect("a length");
    // SAFETY: `path` is writable for `length` units; the handle is open with `PROCESS_QUERY_LIMITED_INFORMATION`.
    let read = unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            PROCESS_NAME_WIN32,
            path.as_mut_ptr(),
            &raw mut length,
        )
    };
    if read == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&path[..usize::try_from(length).expect("a length")]);
    path.rsplit('\\').next().unwrap_or_default().to_owned()
}
