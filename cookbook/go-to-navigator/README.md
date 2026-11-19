# Go to navigator

One chord opens a searchable tree of the window's workspaces, sessions and panes with each agent's status, and Enter jumps there.

Author: [@nickalie](https://github.com/nickalie), after the Go to window [@avlebedinsky](https://github.com/avlebedinsky) proposed in [pull request #28](https://github.com/nickalie/ngterm/pull/28), itself modeled on herdr's navigator.

## What it does

`go-to-navigator.sh` opens a floating page over the session you are in. It lists every workspace of the window with the git branch of its first session, and under each one its sessions on `├─`/`└─` lines, nested as the sidebar nests them: a session's child sessions hang under it, and a split session lists its two panes as `agent · session`. Each pane row shows the agent running there (`terminal` without one), its status (`shell`, `idle`, `working`, `blocked`, `done` or `unknown`) and a `●` on the pane you are in. A counter on top says how many terminals the rows cover, and the two lines under the rows give the selected row's `workspace / session` and its working directory, or a workspace's branch.

Keys:

| key | does |
|---|---|
| ↑ ↓, `j` `k` | move one row; Ctrl+U and Ctrl+D (or Page Up/Down) move eight |
| ← → | jump to the previous or next workspace |
| Home, End or `G` | first pane row, last row |
| `/` | search: type words, each must be in one field (session, label, cwd, agent, the pane's terminal title, or the workspace name and branch); a row kept by its title or cwd shows that text dimmed after its label; Esc leaves the search |
| `b` `w` `i` `d` | show only blocked, working, idle or done panes |
| `t` | show the top-level sessions only, a split one as one row |
| `a`, Backspace | drop the filter (`a` also clears the search) |
| Enter, double-click | go to the row: a workspace, a session, or a pane, focused |
| Esc | close |

The rows stay live while the page is open: a status change or a new session shows up within a couple of seconds. Nothing is closed or changed; the only thing the recipe does is select a workspace or session and focus a pane.

## Requirements

- ngterm 0.1.0 or later: HTML overlay pages with `--js`, `--chromeless` and `--block`, the page bridge's `ngterm.request` and `session.overlay.submit`, and the `tree` fields the rows read (`parent`, `hasSplit`, `splitFocused`, `splitCwd`, `statusPane`, `remoteHost`) all shipped in it.
- `bash`, `jq` and `git`.
- macOS or Linux. The script is bash, so on Windows it runs only from a bash such as Git Bash, which this recipe has not been tried in.

## Setup

Copy the recipe's directory into place, keeping `assets/` beside the script, which reads the page from there:

```sh
mkdir -p ~/.config/ngterm/recipes
cp -R go-to-navigator ~/.config/ngterm/recipes/
chmod +x ~/.config/ngterm/recipes/go-to-navigator/go-to-navigator.sh
```

Add a line to `~/.config/ngterm/keymap.conf`:

```
command "Go to" ctrl+shift+k --error-hud ~/.config/ngterm/recipes/go-to-navigator/go-to-navigator.sh
```

Pick another chord if that one is taken: `ngtermctl keymap list` shows which are. Apply the file with File ▸ Reload Keymap or `ngtermctl keymap reload`. `--error-hud` puts a failure (a missing `jq`, say) on screen for ten seconds instead of only in a notification.

Fired from a chord, the script runs with the app's `PATH` plus the directory of the bundled `ngtermctl`, so a bare `ngtermctl` resolves. A `jq` or `git` your shell profile adds from elsewhere may not: write the absolute path into the script, or wrap the keymap line in `$SHELL -lc '…'`. Set `NGTERMCTL` to the CLI's full path if it sits somewhere unusual.

## Usage

Press the chord, type `/api` to narrow the list to what mentions `api`, or press `b` for the panes waiting on you, move to a row and press Enter. Esc leaves everything as it was.

From a shell inside ngterm, `go-to-navigator.sh` opens it over the selected session. `go-to-navigator.sh page` writes the page with the current tree to a temporary file and prints its path; open that in a browser to check the rows without ngterm (Enter does nothing there).

## How it works

The script reads `ngtermctl tree --json` for the window the chord fired in (`$AGT_WINDOW_ID`), runs `git symbolic-ref` in each local session's directory for the branch names, and builds one self-contained page: the template in `assets/go-to.html`, the model in `assets/go-to.js` and the tree as data, all inlined, because a page opened without `--cwd` has no file access. It opens the page with `session overlay open --html … --js --chromeless --size-percent 80 --block` on the session the chord fired from, and blocks.

The page keeps everything in the browser: rows, search, filters and selection are a pure function of the tree, and every 1.5 seconds it asks `ngterm.request('tree')` for a fresh one, which the bridge answers for the page's own window. Enter answers the page with `session.overlay.submit` and the target (`w:<workspace>`, `s:<session>` or `p:<session>:left|right`), which closes it; the blocked script reads the value and runs `workspace select`, `session select` and, for a pane of a split session, `session focus left|right`. Esc closes the page with `session.overlay.close`, which ends the block with exit 2, and the script does nothing.

Selecting from the script rather than from the page is deliberate: the page sits on the session you started in, and switching away first would leave it to close a page on a session no longer in view.

The agent is read from the pane's foreground process: the first argument whose name is a known agent CLI (`claude`, `codex`, `opencode`, `gemini` and others; the list is `AGENTS` in `assets/go-to.js`), so a wrapper such as `npx claude` still counts. The status is the session's, from the agent-status hooks, and belongs to the pane `statusPane` names (the main pane when it names none). In a split, the other pane reads `unknown` when an agent runs there and `shell` when none does, since ngterm keeps one status per session.

List keys are the letters they type, so `j`, `k` and the filters follow AZERTY, Dvorak or Colemak; on a layout that types no Latin letters they fall back to the key's position on a US keyboard. The search types what the layout types.

`go-to.test.js` checks the rows, filters and keys over a fixed tree: `node --test go-to.test.js`.

## Limits

Nothing here closes or deletes anything; Enter only changes which session is selected and which pane has focus.

A session attached from another machine runs `ssh` in its pane as far as this machine can tell, so its panes read `terminal` even when an agent runs there; their status is still right. Such a session gets no branch either.

Branches are read once, when the page opens: a session created while it is open has none until the next time.

The page opens over the selected session, so a session that already shows an overlay or page refuses it (`overlay already open`), and `--error-hud` shows that error.

The agent list is a fixed set of names; an agent CLI it does not know reads as `terminal` until you add it to `AGENTS`.
