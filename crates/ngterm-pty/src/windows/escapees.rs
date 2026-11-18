//! The processes that leave a child's job, taken back in. A packaged app (an MSIX package, such as PowerShell from
//! the Microsoft Store) starts each program outside its package's runtime, and Windows then creates that program
//! outside every job, a kill-on-close job without `JOB_OBJECT_LIMIT_BREAKAWAY_OK` included. A program that a pane's
//! process starts is outside its job only so: the job permits no breakaway, and a process another one creates with
//! the pane's process as its parent joins that parent's job. So a process that a process of the job created, but
//! which is not in the job itself, escaped it, and is assigned to it. The job then ends it with the rest of the tree,
//! lists it as the pane's foreground and keeps it across a handoff.
//!
//! The process table is a snapshot, and an id in it can name another process by the time it is read: one that took
//! the id of a process that ended since. So the table only names the candidates. A [`Tree`] holds each process of
//! the job open, and an open process keeps its id from its creation on. A candidate is opened in turn, and its
//! creator's id is read through that handle (`NtQueryInformationProcess`), not from the snapshot. A candidate
//! whose creator's id is that of a held process, and which was created after that process, was created by it. Any
//! other process is left alone.
//!
//! [`Tree::adopt`] does one pass: [`super::Killer::kill_tree`] before it terminates the job and again once the tree
//! ended (what a process of the tree started as it ended), and the job's last close before it kills the tree. The
//! close cannot wait for the tree to end (after a handoff, another process holds the job and the tree runs on), so
//! what a process of the tree starts outside the job between that pass and the kill stays out.
//! [`watch`] repeats the pass every [`TICK`] for the job of a packaged program, so what it starts joins the job
//! before the pane ends, the app exits or a program reads the pane's foreground. A program that a short-lived
//! escaped process starts and leaves behind between two passes stays outside the job: its parent is gone, and the
//! process table names it no more.

use std::collections::HashSet;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, IsProcessInJob};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, WaitForSingleObject,
};

use super::Job;
use crate::actor::lock;

/// How often a watched job takes back what escaped it.
pub(super) const TICK: Duration = Duration::from_millis(250);

/// How long [`super::Killer::kill_tree`] waits for the terminated tree to end before its last pass: a terminated
/// process ends in milliseconds, and until then it can still start one more.
pub(super) const ENDING: Duration = Duration::from_secs(1);

/// The rights a process of the tree is held with: to read its creation time, and to wait for its end.
const HELD: u32 = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;

/// The rights a candidate is opened with: [`HELD`], and the rights its assignment to a job needs.
const CANDIDATE: u32 = HELD | PROCESS_SET_QUOTA | PROCESS_TERMINATE;

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

/// A process of a job's tree, held open: no other process can get its id while the handle is open.
#[derive(Debug)]
struct Held {
    pid: u32,
    /// When it was created, in 100 ns units since 1601.
    started: u64,
    process: OwnedHandle,
}

/// The processes of a job's tree, held open from the time they are found, so that the id of each names it alone.
#[derive(Debug)]
pub(super) struct Tree {
    held: Vec<Held>,
}

impl Tree {
    /// Each process that `job` holds now, opened: one that cannot be opened (another user's, an elevated one) or
    /// that holds the id of one that left the job is not held.
    pub(super) fn of(job: &OwnedHandle) -> Self {
        let held = super::job_process_ids(job)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pid| {
                let process = open(pid, HELD)?;
                if !in_job(&process, job)? {
                    return None;
                }
                Some(Held {
                    pid,
                    started: started(&process)?,
                    process,
                })
            })
            .collect();
        Self { held }
    }

    /// Assign to `job` each process of `processes` that escaped it: one that a held process created, that is not in
    /// the job (a breakaway the system made). One taken back is held, and counts as a parent in its turn, as what it
    /// started before then escaped too. A process that cannot be opened with the rights the assignment needs (another
    /// user's, an elevated one) stays where it is.
    pub(super) fn adopt(&mut self, job: &OwnedHandle, processes: &[Entry]) {
        let mut known: HashSet<u32> = self.held.iter().map(|held| held.pid).collect();
        let mut next = 0;
        while let Some(parent) = self.held.get(next) {
            let (parent, parent_started) = (parent.pid, parent.started);
            next += 1;
            for entry in processes {
                if entry.parent != parent || entry.pid == parent || known.contains(&entry.pid) {
                    continue;
                }
                if let Some(child) = take_back(job, entry.pid, parent, parent_started) {
                    known.insert(child.pid);
                    self.held.push(child);
                }
            }
        }
    }

    /// Wait up to `limit` for each held process to end.
    pub(super) fn wait_ended(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        for held in &self.held {
            let left = deadline.saturating_duration_since(Instant::now());
            let millis = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
            // SAFETY: waits on a process handle held open with `PROCESS_SYNCHRONIZE`; no memory is passed.
            unsafe { WaitForSingleObject(held.process.as_raw_handle(), millis) };
        }
    }
}

/// Process `pid` held, after it is assigned to `job` unless it is in it already, when `parent` (a process the caller
/// holds, created at `parent_started`) created it. A process that has the id now but that `parent` did not create
/// (it got the id after the caller's snapshot, from a process that ended) is left alone.
fn take_back(job: &OwnedHandle, pid: u32, parent: u32, parent_started: u64) -> Option<Held> {
    let process = open(pid, CANDIDATE)?;
    if creator(&process)? != parent {
        return None;
    }
    let started = started(&process)?;
    if started < parent_started {
        return None;
    }
    if !in_job(&process, job)? {
        // SAFETY: both handles are open, the process one with the rights an assignment needs.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) } == 0 {
            tracing::debug!(pid, err = %std::io::Error::last_os_error(), "a process that left its pane's job stays out");
            return None;
        }
    }
    Some(Held { pid, started, process })
}

/// Whether `process` is in `job`; `None` when that cannot be read.
fn in_job(process: &OwnedHandle, job: &OwnedHandle) -> Option<bool> {
    let mut inside = 0;
    // SAFETY: both handles are open; the result goes to a live local.
    let read = unsafe { IsProcessInJob(process.as_raw_handle(), job.as_raw_handle(), &raw mut inside) };
    (read != 0).then_some(inside != 0)
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
    lock(&watcher.jobs).push(Arc::downgrade(job));
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
            Tree::of(&job.handle).adopt(&job.handle, &processes);
        }
        // the last holder may be this thread: the job's close then runs here, its last pass included
        drop(jobs);
        std::thread::sleep(TICK);
    }
}

/// The watched jobs still open, once there is one; the closed ones are forgotten.
fn live_jobs(watcher: &Watcher) -> Vec<Arc<Job>> {
    let mut jobs = lock(&watcher.jobs);
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

/// The id of the process that created `process` (`InheritedFromUniqueProcessId`), read through its handle; `None`
/// when it cannot be read.
fn creator(process: &OwnedHandle) -> Option<u32> {
    let mut basic = PROCESS_BASIC_INFORMATION::default();
    let size = u32::try_from(size_of::<PROCESS_BASIC_INFORMATION>()).ok()?;
    // SAFETY: the handle is open with `PROCESS_QUERY_LIMITED_INFORMATION`, and `basic` is a live local of exactly
    // `size` bytes.
    let status = unsafe {
        NtQueryInformationProcess(
            process.as_raw_handle(),
            ProcessBasicInformation,
            (&raw mut basic).cast(),
            size,
            std::ptr::null_mut(),
        )
    };
    if status < 0 {
        return None;
    }
    u32::try_from(basic.InheritedFromUniqueProcessId).ok()
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
