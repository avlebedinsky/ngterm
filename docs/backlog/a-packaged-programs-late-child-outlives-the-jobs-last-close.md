---
worth: yes
where: crates/ngterm-pty/src/windows/mod.rs:96
added: 2026-11-18
---
# A child that a packaged program starts as the job's last close ends the tree stays alive

On Windows, `Job::drop` (the last close of a pane's kill-on-close job) does one pass of `escapees::Tree::adopt`, and
then the close of the handle kills the tree. A packaged program (PowerShell from the Microsoft Store) creates its
children outside every job. When such a program starts a child after that pass and before the kill, the child is
not in the job and the kill does not end it. After the program ended, no later pass can find the child.

`Killer::kill_tree` does not have this gap: it holds the tree's processes open, terminates the job, waits for the
tree to end, then takes back and terminates what a process of the tree started outside the job. The close cannot do
the same in place. After a handoff, another process holds the job, the close does not end the tree, and a wait for
the tree would block the close.

The race needs a command to start in the few milliseconds of the close, and it occurs only when nothing called
`kill_tree` first (the shell ended by itself and left background programs, or the app dropped a pane without a
kill). The result is a stray process, not lost data.

Possible fix: let the close give the held tree (`escapees::Tree`) to the watch thread. After the close, that thread
waits for the held processes. If they end (the close killed the tree), it takes a new snapshot and terminates each
process that a held process created (the same identity check as `Tree::adopt`). If they do not end in some seconds
(another process holds the job), it drops them.

Found in the revmux review of task 537 (Store PowerShell sessions), round 1, finding 2. The fix of that round closed
the gap for `kill_tree` only.
