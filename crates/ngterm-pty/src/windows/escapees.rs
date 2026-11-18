//! The processes that leave a child's job, taken back in. A packaged app (an MSIX package, such as PowerShell from
//! the Microsoft Store) starts each program outside its package's runtime, and Windows then creates that program
//! outside every job, a kill-on-close job without `JOB_OBJECT_LIMIT_BREAKAWAY_OK` included. A program that a pane's
//! process starts is outside its job only so: the job permits no breakaway, and a process another one creates with
//! the pane's process as its parent joins that parent's job. So a process whose parent is in the job, created after
//! that parent, but which is not in the job itself, escaped it, and is assigned to it. The job then ends it with the
//! rest of the tree, lists it as the pane's foreground and keeps it across a handoff.
//!
//! [`adopt`] does one pass: [`super::Killer::kill_tree`] before it terminates the job, and the job's last close
//! before it kills the tree. [`watch`] repeats the pass every [`TICK`] for the job of a packaged program, so what it
//! starts joins the job before the pane ends, the app exits or a program reads the pane's foreground. A program that
//! a short-lived escaped process starts and leaves behind between two passes stays outside the job: its parent is
//! gone, and the process table names it no more.

use std::collections::HashSet;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, IsProcessInJob};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

use super::Job;

/// How often a watched job takes back what escaped it.
pub(super) const TICK: Duration = Duration::from_millis(250);

/// A process the system lists: its id and the id of the process that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) pid: u32,
    pub(super) parent: u32,
}

/// Every process now, from a tool-help snapshot; empty when no snapshot can be taken.
pub(super) fn processes() -> Vec<Entry> {
    // SAFETY: a snapshot of the process list; no memory is passed.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    // SAFETY: a fresh handle nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX),
        ..PROCESSENTRY32W::default()
    };
    let mut listed = Vec::new();
    // SAFETY: `entry` is a live local of the size its `dwSize` gives; the snapshot handle is open.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        listed.push(Entry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    listed
}

/// Assign to `job` each process of `processes` that escaped it: one created after its parent, a process of the
/// job, that is not in the job (a breakaway the system made). One taken back counts as a parent in its turn, as
/// whatever it started before then escaped too. A process that cannot be opened with the rights the assignment
/// needs (another user's, an elevated one) stays where it is.
pub(super) fn adopt(job: &OwnedHandle, processes: &[Entry]) {
    let Some(members) = super::job_process_ids(job) else {
        return;
    };
    let mut known: HashSet<u32> = members.iter().copied().collect();
    let mut parents = members;
    while let Some(parent) = parents.pop() {
        let children: Vec<u32> = processes
            .iter()
            .filter(|entry| entry.parent == parent && entry.pid != parent && !known.contains(&entry.pid))
            .map(|entry| entry.pid)
            .collect();
        if children.is_empty() {
            continue;
        }
        let Some(parent_started) =
            open(parent, PROCESS_QUERY_LIMITED_INFORMATION).and_then(|process| started(&process))
        else {
            continue;
        };
        for child in children {
            if take_back(job, child, parent_started) {
                known.insert(child);
                parents.push(child);
            }
        }
    }
}

/// Assign process `pid` to `job` unless it is in it already; whether it is in the job now. A process created
/// before `parent_started` names its parent by an id an earlier process held, and is left alone.
fn take_back(job: &OwnedHandle, pid: u32, parent_started: u64) -> bool {
    let Some(process) = open(
        pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA | PROCESS_TERMINATE,
    ) else {
        return false;
    };
    if started(&process).is_none_or(|child| child < parent_started) {
        return false;
    }
    let mut inside = 0;
    // SAFETY: both handles are open; the result goes to a live local.
    if unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &raw mut inside) } == 0 {
        return false;
    }
    if inside != 0 {
        return true;
    }
    // SAFETY: both handles are open, the process one with the rights an assignment needs.
    let assigned = unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) } != 0;
    if !assigned {
        tracing::debug!(pid, err = %std::io::Error::last_os_error(), "a process that left its pane's job stays out");
    }
    assigned
}

/// Whether `process` runs with a package's identity (an MSIX package, such as PowerShell from the Microsoft Store):
/// what it starts then leaves the job.
pub(super) fn packaged(process: std::os::windows::io::BorrowedHandle<'_>) -> bool {
    let mut length = 0_u32;
    // SAFETY: with a length of zero and no buffer the call only reports the name's length into a live local.
    let status = unsafe { GetPackageFamilyName(process.as_raw_handle(), &raw mut length, std::ptr::null_mut()) };
    status == ERROR_INSUFFICIENT_BUFFER
}

/// Take back what escapes `job` every [`TICK`], for as long as something else holds it open, on one thread for
/// every job watched in this process.
pub(super) fn watch(job: &Arc<Job>) {
    let watcher = watcher();
    watcher
        .jobs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::downgrade(job));
    watcher.added.notify_one();
}

/// The jobs [`watch`] was given, and what wakes its thread when one comes while it has none.
#[derive(Debug, Default)]
struct Watcher {
    jobs: Mutex<Vec<Weak<Job>>>,
    added: Condvar,
}

fn watcher() -> &'static Watcher {
    static WATCHER: OnceLock<Watcher> = OnceLock::new();
    WATCHER.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("ngterm-pty-job-watch".into())
            .spawn(|| watch_forever(watcher()));
        if let Err(err) = spawned {
            tracing::warn!(%err, "no thread takes back what leaves a packaged program's job");
        }
        Watcher::default()
    })
}

fn watch_forever(watcher: &Watcher) {
    loop {
        let jobs = live_jobs(watcher);
        let processes = processes();
        for job in &jobs {
            adopt(&job.handle, &processes);
        }
        // the last holder may be this thread: the job's close then runs here, its last pass included
        drop(jobs);
        std::thread::sleep(TICK);
    }
}

/// The watched jobs still open, once there is one; the closed ones are forgotten.
fn live_jobs(watcher: &Watcher) -> Vec<Arc<Job>> {
    let mut jobs = watcher.jobs.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        jobs.retain(|job| job.strong_count() > 0);
        let live: Vec<Arc<Job>> = jobs.iter().filter_map(Weak::upgrade).collect();
        if !live.is_empty() {
            return live;
        }
        jobs = watcher.added.wait(jobs).unwrap_or_else(PoisonError::into_inner);
    }
}

/// Process `pid` with the `access` rights; `None` when it is gone or they are refused.
fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
    // SAFETY: opens a process by id; no memory is passed.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    // SAFETY: a fresh handle nothing else owns.
    (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// When `process` was created, in 100 ns units since 1601.
fn started(process: &OwnedHandle) -> Option<u64> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: the handle is open with `PROCESS_QUERY_LIMITED_INFORMATION`; the four times go to live locals.
    let read = unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &raw mut *created,
            &raw mut *exited,
            &raw mut *kernel,
            &raw mut *user,
        )
    };
    (read != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
