//! The resume of a program created suspended, over a snapshot of the system's threads: Windows keeps no handle to
//! the program's main thread, so `sys` lists every thread and resumes the program's own. Apart from the Win32 calls,
//! so a fake snapshot tests it on every OS.

use std::io;

/// A thread a snapshot of the system's threads names, under the pid of the process it ran in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ListedThread {
    pub(super) id: u32,
    pub(super) owner: u32,
}

/// A listed thread, opened by its id.
pub(super) trait OpenedThread {
    /// Whether the thread has ended. An ended thread can stay in a snapshot after its process reads as exited, and
    /// it still opens and resumes without an error.
    fn ended(&self) -> bool;
    /// The pid of the process the thread runs in now: a listed id can name a newer thread once the listed one is
    /// gone.
    fn owner(&self) -> Option<u32>;
    /// Resumes the thread; `false` when the system refuses.
    fn resume(&self) -> bool;
}

/// Resumes each thread of `pid` that `listed` names and that has not ended. A program with no such thread cannot be
/// resumed: it has exited, or it is not there.
pub(super) fn resume_listed<T: OpenedThread>(
    pid: u32,
    listed: impl IntoIterator<Item = ListedThread>,
    mut open: impl FnMut(u32) -> Option<T>,
) -> io::Result<()> {
    let mut resumed = 0_usize;
    for thread in listed.into_iter().filter(|thread| thread.owner == pid) {
        let Some(opened) = open(thread.id) else {
            continue;
        };
        if !opened.ended() && opened.owner() == Some(pid) && opened.resume() {
            resumed += 1;
        }
    }
    if resumed == 0 {
        return Err(io::Error::other("the program's thread could not be resumed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    const PROGRAM: u32 = 40;
    const OTHER: u32 = 41;

    /// A thread as the fake system holds it.
    #[derive(Debug, Clone, Copy)]
    struct FakeThread {
        owner: u32,
        ended: bool,
        refuses: bool,
    }

    impl FakeThread {
        fn running_in(owner: u32) -> Self {
            Self {
                owner,
                ended: false,
                refuses: false,
            }
        }
    }

    /// The fake system: its threads by id, and the ids each resume reached.
    #[derive(Default)]
    struct System {
        threads: BTreeMap<u32, FakeThread>,
        resumed: RefCell<Vec<u32>>,
    }

    struct Opened<'a> {
        id: u32,
        thread: FakeThread,
        system: &'a System,
    }

    impl OpenedThread for Opened<'_> {
        fn ended(&self) -> bool {
            self.thread.ended
        }

        fn owner(&self) -> Option<u32> {
            Some(self.thread.owner)
        }

        fn resume(&self) -> bool {
            self.system.resumed.borrow_mut().push(self.id);
            !self.thread.refuses
        }
    }

    impl System {
        fn with(threads: &[(u32, FakeThread)]) -> Self {
            Self {
                threads: threads.iter().copied().collect(),
                ..Self::default()
            }
        }

        /// Resumes `pid` over `listed`, opening each id as the system holds it now.
        fn resume(&self, pid: u32, listed: &[(u32, u32)]) -> Result<(), String> {
            let listed = listed.iter().map(|&(id, owner)| ListedThread { id, owner });
            resume_listed(pid, listed, |id| {
                self.threads.get(&id).map(|&thread| Opened {
                    id,
                    thread,
                    system: self,
                })
            })
            .map_err(|error| error.to_string())
        }

        fn resumed(&self) -> Vec<u32> {
            self.resumed.borrow().clone()
        }
    }

    const NOT_RESUMED: &str = "the program's thread could not be resumed";

    #[test]
    fn a_suspended_programs_threads_are_resumed_and_no_other_process_is_touched() {
        let system = System::with(&[
            (1, FakeThread::running_in(PROGRAM)),
            (2, FakeThread::running_in(OTHER)),
            (3, FakeThread::running_in(PROGRAM)),
        ]);
        assert_eq!(
            system.resume(PROGRAM, &[(1, PROGRAM), (2, OTHER), (3, PROGRAM)]),
            Ok(())
        );
        assert_eq!(system.resumed(), [1, 3]);
    }

    /// The snapshot still names the thread of a program that has exited, and the thread still opens.
    #[test]
    fn a_program_whose_listed_thread_has_ended_cannot_be_resumed() {
        let ended = FakeThread {
            ended: true,
            ..FakeThread::running_in(PROGRAM)
        };
        let system = System::with(&[(1, ended)]);
        assert_eq!(system.resume(PROGRAM, &[(1, PROGRAM)]), Err(NOT_RESUMED.to_owned()));
        assert_eq!(system.resumed(), [0_u32; 0]);
    }

    /// The listed thread is gone and its id names a thread of another process by the time it opens.
    #[test]
    fn a_listed_id_that_now_names_another_process_thread_is_left_alone() {
        let system = System::with(&[(1, FakeThread::running_in(OTHER))]);
        assert_eq!(system.resume(PROGRAM, &[(1, PROGRAM)]), Err(NOT_RESUMED.to_owned()));
        assert_eq!(system.resumed(), [0_u32; 0]);
    }

    #[test]
    fn a_program_with_no_thread_that_opens_or_resumes_cannot_be_resumed() {
        let refuses = FakeThread {
            refuses: true,
            ..FakeThread::running_in(PROGRAM)
        };
        let system = System::with(&[(2, refuses)]);
        assert_eq!(system.resume(PROGRAM, &[]), Err(NOT_RESUMED.to_owned()));
        assert_eq!(
            system.resume(PROGRAM, &[(1, PROGRAM), (2, PROGRAM)]),
            Err(NOT_RESUMED.to_owned())
        );
        assert_eq!(system.resumed(), [2]);
    }
}
