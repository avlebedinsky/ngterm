//! The state directory's owner: one process holds an exclusive lock on [`LOCK_FILE_NAME`] in the state directory for
//! its whole life, and a second launch on the same directory exits before it reads or writes anything there.
//!
//! Ported from upstream's `agtermCore/Sources/agtermCore/StateDirectoryLock.swift` and `agtermApp.claimStateDirectory`
//! (#749): every store save rewrites its whole file from memory, so a second process loaded from the same directory
//! would overwrite the owner's later changes with its stale copy. Upstream takes `flock(LOCK_EX | LOCK_NB)` on an
//! `O_CLOEXEC` descriptor; here [`File::try_lock`] takes the same `flock` on Linux and macOS and
//! `LockFileEx(EXCLUSIVE | FAIL_IMMEDIATELY)` on Windows, on a handle std opens close-on-exec (not inheritable on
//! Windows), so no shell, session server or worker the owner spawns keeps the lock past its exec. The kernel releases
//! it when the process ends, a crash or a kill included. The file is created, never truncated and never removed: a
//! process locking a fresh inode would exclude nobody.
//!
//! On Unix, drop unlocks before the file closes, so a child with a duplicate descriptor cannot retain the lock.
//! Windows handles are not inherited, so closure of the owner handle is enough.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;

/// The lock file inside the state directory.
pub const LOCK_FILE_NAME: &str = "ngterm.lock";

/// What [`StateDirectoryLock::acquire`] found. Only [`Outcome::Owned`] grants the directory; [`Outcome::Unavailable`]
/// proves nothing about whether another process owns it.
#[derive(Debug)]
pub enum Outcome {
    /// This process owns the directory while the lock lives.
    Owned(StateDirectoryLock),
    /// Another process holds the lock.
    HeldByAnother,
    /// The lock could not be taken: `cannot create <dir>: …`, `cannot open <path>: …` or `cannot lock <path>: …`.
    Unavailable(String),
}

impl Outcome {
    /// The lock when owned, else the reason a launch on `directory` stops: upstream's two refusals, under this build's
    /// CLI name.
    ///
    /// # Errors
    ///
    /// `another instance is already running on <dir>; control it with ngtermctl` when another process holds the lock,
    /// `cannot claim the state directory: <detail>` when it could not be taken.
    pub fn claim(self, directory: &Path) -> Result<StateDirectoryLock, String> {
        match self {
            Self::Owned(lock) => Ok(lock),
            Self::HeldByAnother => Err(format!(
                "another instance is already running on {}; control it with ngtermctl",
                directory.display()
            )),
            Self::Unavailable(detail) => Err(format!("cannot claim the state directory: {detail}")),
        }
    }
}

/// Ownership of a state directory, held until dropped (for the app, the whole process).
#[derive(Debug)]
pub struct StateDirectoryLock {
    _file: File,
}

#[cfg(unix)]
impl Drop for StateDirectoryLock {
    fn drop(&mut self) {
        let Self { _file: file } = self;
        let _ = file.unlock();
    }
}

impl StateDirectoryLock {
    /// Creates a missing `directory` and takes the exclusive lock on its [`LOCK_FILE_NAME`] without waiting.
    #[must_use]
    pub fn acquire(directory: &Path) -> Outcome {
        if let Err(error) = fs::create_dir_all(directory) {
            return Outcome::Unavailable(format!("cannot create {}: {error}", directory.display()));
        }
        let path = directory.join(LOCK_FILE_NAME);
        let file = match open(&path) {
            Ok(file) => file,
            Err(error) => return Outcome::Unavailable(format!("cannot open {}: {error}", path.display())),
        };
        match file.try_lock() {
            Ok(()) => Outcome::Owned(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Outcome::HeldByAnother,
            Err(TryLockError::Error(error)) => Outcome::Unavailable(format!("cannot lock {}: {error}", path.display())),
        }
    }
}

/// Opens (creating, never truncating) the lock file, owner-only on Unix as upstream's `0o600`.
fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(outcome: Outcome) -> Option<StateDirectoryLock> {
        match outcome {
            Outcome::Owned(lock) => Some(lock),
            Outcome::HeldByAnother | Outcome::Unavailable(_) => None,
        }
    }

    #[test]
    fn acquire_creates_a_missing_directory_and_owns_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested/state");
        let lock = owned(StateDirectoryLock::acquire(&dir));
        assert!(lock.is_some());
        assert!(dir.join(LOCK_FILE_NAME).is_file());
    }

    // A second handle of this process conflicts as another process's would: flock locks belong to the open file
    // description, LockFileEx locks to the handle.
    #[test]
    fn a_second_acquire_while_the_first_is_held_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let first = owned(StateDirectoryLock::acquire(tmp.path())).expect("the first acquire owns the directory");
        let second = StateDirectoryLock::acquire(tmp.path());
        assert!(matches!(second, Outcome::HeldByAnother), "{second:?}");
        assert_eq!(
            second.claim(tmp.path()).unwrap_err(),
            format!(
                "another instance is already running on {}; control it with ngtermctl",
                tmp.path().display()
            )
        );
        drop(first);
    }

    #[cfg(unix)]
    #[test]
    fn releasing_the_lock_frees_it_while_a_duplicate_descriptor_stays_open() {
        let tmp = tempfile::tempdir().unwrap();
        let first = StateDirectoryLock::acquire(tmp.path()).claim(tmp.path()).unwrap();
        let StateDirectoryLock { _file: file } = &first;
        let duplicate = file.try_clone().unwrap();
        assert!(matches!(
            StateDirectoryLock::acquire(tmp.path()),
            Outcome::HeldByAnother
        ));

        drop(first);

        let next = StateDirectoryLock::acquire(tmp.path()).claim(tmp.path()).unwrap();
        drop(duplicate);
        assert!(matches!(
            StateDirectoryLock::acquire(tmp.path()),
            Outcome::HeldByAnother
        ));
        drop(next);
    }

    #[test]
    fn releasing_the_lock_lets_the_next_acquire_own_it_and_keeps_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let first = owned(StateDirectoryLock::acquire(tmp.path()));
        assert!(first.is_some());
        drop(first);
        assert!(tmp.path().join(LOCK_FILE_NAME).is_file());
        assert!(owned(StateDirectoryLock::acquire(tmp.path())).is_some());
    }

    #[test]
    fn a_directory_path_held_by_a_file_is_unavailable_not_held() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("state");
        fs::write(&file, "x").unwrap();
        let outcome = StateDirectoryLock::acquire(&file);
        let Outcome::Unavailable(detail) = &outcome else {
            panic!("a state directory that cannot be created must be unavailable: {outcome:?}");
        };
        let prefix = format!("cannot create {}: ", file.display());
        assert!(detail.starts_with(&prefix), "{detail}");
        let expected = format!("cannot claim the state directory: {detail}");
        assert_eq!(outcome.claim(&file).unwrap_err(), expected);
    }

    #[test]
    fn the_lock_file_keeps_its_contents() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(LOCK_FILE_NAME), "kept").unwrap();
        let lock = owned(StateDirectoryLock::acquire(tmp.path()));
        assert!(lock.is_some());
        // read after the release: LockFileEx's lock is mandatory, so on Windows any other handle's read of the locked
        // file fails while it is held
        drop(lock);
        assert_eq!(fs::read_to_string(tmp.path().join(LOCK_FILE_NAME)).unwrap(), "kept");
    }

    #[test]
    fn a_directory_in_place_of_the_lock_file_cannot_be_claimed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LOCK_FILE_NAME);
        fs::create_dir(&path).unwrap();
        let Outcome::Unavailable(detail) = StateDirectoryLock::acquire(tmp.path()) else {
            panic!("a directory cannot be opened as the lock file");
        };
        assert!(detail.starts_with(&format!("cannot open {}: ", path.display())));
        assert!(path.is_dir());
        assert_eq!(
            Outcome::Unavailable(detail.clone()).claim(tmp.path()).unwrap_err(),
            format!("cannot claim the state directory: {detail}")
        );
    }

    #[test]
    fn claiming_an_owned_directory_keeps_its_lock_alive() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = StateDirectoryLock::acquire(tmp.path()).claim(tmp.path()).unwrap();
        assert!(matches!(
            StateDirectoryLock::acquire(tmp.path()),
            Outcome::HeldByAnother
        ));
        drop(lock);
    }
}
