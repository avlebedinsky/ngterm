# Contributing a recipe

Send a workflow you actually run. A script written for the collection rather than for your own day is the wrong kind of submission: if it has not driven your own sessions for a while, it has not been tested where it matters.

Open a pull request against `main` with the recipe directory, its README, and the index row, all in one change.

## Layout

One directory per recipe, kebab-case, named after what the recipe does rather than after its script (`go-to-navigator`, not `ngt-goto`). It holds a `README.md` and the supporting files listed below, and nothing more. A recipe that is only instructions, a set of `ngtermctl` commands or `keymap.conf` lines the reader types in, is a README alone. Anything else it needs is one of these:

- **Scripts**, named by language as described below.
- **A config file** the reader copies into place, named for its purpose, when the recipe is configuration rather than code.
- **An agent skill**, whether a chord hands work to it or the reader invokes it directly, as `SKILL.md`, or one `SKILL-<agent>.md` per agent for a recipe that drives more than one. Keep it to what the recipe needs.
- **Data files** the scripts or the skill read at run time, such as page templates, in a subdirectory (`assets/`).
- **Tests** of the recipe's own logic, beside the README.
- **A screenshot** for a recipe that puts something on screen, beside the README and embedded right under the *Author* line: `<img src="screenshot.png" alt="what it shows" width="283">`, a 2x capture shown at half its width.

*Setup* says where the files go. A script that finds its data by a path relative to itself keeps it in the subdirectory it names, and *Setup* says they must be installed together.

Name scripts by their language, because the extension decides what CI does with them (`scripts/lint.sh`, run by the `lint` workflow):

- `.sh` is POSIX or bash. CI runs the pinned `shellcheck` over every one, and it has to be clean.
- `.js` runs in an overlay page or under Node. CI parses every one with `node --check`, and runs each `*.test.js` with `node --test`. A parse is not a lint, so read it over before sending.

A recipe in another language is welcome, but say so in the pull request: nothing gates an extension CI does not know, and the reader has to trust it entirely on review.

Every script carries a shebang, except a `.js` file a page inlines. A script the reader invokes directly is committed with the executable bit set; a file that is sourced or inlined rather than invoked is committed without it, and *Setup* never tells the reader to run it.

## The README template

Six headings, exactly these, in this order. CI checks that all six are present:

```markdown
# <Title>

<one-line summary>

Author: [@<login>](https://github.com/<login>)

## What it does
## Requirements
## Setup
## Usage
## How it works
## Limits
```

**Requirements** names the minimum ngterm version the recipe needs, written as "X or later", plus any external tool it calls and the platforms it runs on. Work out that minimum from the commands and flags the recipe uses rather than naming the version you happen to run, and say in the same line what shipped in it.

**Setup** is the exact steps, including where the files go and how they are invoked. Anything machine-specific is a variable the reader sets, named and explained here.

**How it works** explains the mechanism, not the code line by line. The gotcha that cost you an hour belongs here.

**Limits** states destructive behavior in plain words. If the recipe closes sessions, deletes workspaces, or kills a running shell, say so in a sentence the reader cannot skim past. Known caveats that are not destructive go here too.

The **Author** line names who wrote the recipe, by GitHub login linked to the profile, and CI checks that it is there. When you port someone else's work, name them and link where you found it on the same line.

## Index row

Add a row to [README.md](README.md) in the same pull request:

| recipe | what it does | needs | author |
|---|---|---|---|
| [go-to-navigator](go-to-navigator/) | one chord opens a searchable tree of the window's sessions | 0.1.0, jq, git | [@nickalie](https://github.com/nickalie) |

The *author* column repeats the README's Author line. The index groups recipes by what the reader is trying to do, alphabetical within each group:

- **Workspaces and projects** — arranging and moving between workspaces, windows and sessions.
- **Sessions across restarts** — a tab coming back to what it was running.
- **Agent status and workflows** — reporting what an agent is doing, or an agent workflow driven from a chord or a skill.
- **Panes, pickers and input** — splits, overlays, dashboards, and getting text into the shell.

Add the group's heading and table when the first recipe for it arrives. If none fits, say so in the pull request: a new group is a fair outcome.

CI compares the directory set against the index in both directions, so a directory with no row fails, and a row pointing at a directory that does not exist fails too. The check matches the link by its trailing slash, so keep it.

## Rules for the scripts

Call the CLI through an overridable variable, `NGTERMCTL=${NGTERMCTL:-ngtermctl}`, so a reader whose binary sits somewhere unusual can point at it without editing the script. The exception is a recipe made of `keymap.conf` lines, which use bare command names.

Address the app that ran you: a script started from a chord or the palette passes `--socket "$AGT_SOCKET"`, and one started from a session shell `--socket "$NGTERM_SOCKET"`, since `ngtermctl` reads neither on its own.

Nothing personal reaches a committed file. No absolute paths under a home directory, no host names, IP addresses, user names, or internal project and service names. Replace each with a variable the reader sets, documented in *Setup*, with a neutral default. This applies to comments and example output as much as to code.

Escape anything outside you put into a page (a session name, a title, a path): a page opened with `--js` can run any `ngtermctl` command. Set it as text, never as HTML.

Keep a recipe standing on its own. If two recipes share a gotcha, both explain it, rather than one pointing at the other.

## What review looks like

Every recipe is read before it is merged, for three things: whether its commands, flags, and JSON paths are right against the current control API; what it destroys, and whether the README says so; and whether anything private came along with it. Nothing is executed against a live ngterm during review.

## License

Recipes ship under the license that covers the rest of the repository. Send only work you are free to license that way, and say where a ported script came from.
