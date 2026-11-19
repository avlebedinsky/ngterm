//! The branch checked out where a directory lies, read from the repository's `HEAD` with no `git` process: the
//! repository's root is the nearest directory up from it whose `.git` (a directory, or a worktree's or submodule's
//! `gitdir:` file) holds a `HEAD`.
//!
//! herdr: `src/workspace/git/discovery.rs` (`git_branch`, `git_repo_root`, `git_dir_for_repo_root`,
//! `read_git_ref_file`, `parse_git_head_branch`) at `HERDR_REV` (`build/pins.env`), licensed under the Apache License
//! 2.0, modified: no `git symbolic-ref` for a reftable repository, and a bare repository is not a root.

use std::io::Read as _;
use std::path::{Path, PathBuf};

const MAX_HEAD_BYTES: u64 = 64 * 1024;

/// The branch `HEAD` names under `refs/heads/` in the repository holding `cwd`; `None` outside one and on a detached
/// `HEAD`.
#[must_use]
pub fn git_branch(cwd: &Path) -> Option<String> {
    let git_dir = git_dir(&repo_root(cwd)?)?;
    let head = read_small(&git_dir.join("HEAD"))?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?;
    (!branch.is_empty()).then(|| branch.to_owned())
}

fn repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if git_dir(&current).is_some_and(|git_dir| git_dir.join("HEAD").is_file()) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn git_dir(root: &Path) -> Option<PathBuf> {
    let git = root.join(".git");
    if git.is_dir() {
        return Some(git);
    }
    let link = read_small(&git)?;
    let target = Path::new(link.trim().strip_prefix("gitdir:")?.trim());
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    })
}

fn read_small(path: &Path) -> Option<String> {
    let mut contents = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_HEAD_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    u64::try_from(contents.len())
        .is_ok_and(|length| length <= MAX_HEAD_BYTES)
        .then_some(contents)
}
