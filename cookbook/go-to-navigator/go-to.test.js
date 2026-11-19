#!/usr/bin/env node
"use strict";
// The navigator's rows and keys over a fixed tree, and the script against a stand-in ngtermctl:
// `node --test go-to.test.js` (the script tests need bash, jq and git).
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { Navigator, agentOf, workspacesOf, rows } = require("./assets/go-to.js");

const tree = {
  workspaces: [
    {
      id: "W1", name: "api",
      sessions: [
        { id: "S1", name: "server", cwd: "/src/api", foreground: ["npx", "claude"], status: "active", statusPane: "left" },
        { id: "S2", name: "review", cwd: "/src/api", hasSplit: true, splitCwd: "/src/api/docs",
          foreground: ["codex"], status: "blocked", statusPane: "left", active: true, splitFocused: true },
        { id: "S3", name: "worker", cwd: "/src/api/w", parent: "S2", foreground: ["claude"], status: "completed" },
        { id: "S4", name: "report", parent: "S2", content: "page" },
      ],
    },
    {
      id: "W2", name: "infra",
      sessions: [{ id: "S5", name: "shell", cwd: "/srv", remoteHost: "box" }],
    },
  ],
};
const branches = { "/src/api": "main", "/srv": "ignored-for-remote" };

const lines = list => list.map(r => `${r.tree.join("")}${r.label}${r.pane ? ` [${r.pane.agent} ${r.pane.status}]` : ""}`);
const press = (nav, key, extra = {}) => nav.key({ key, code: "", shift: false, ctrl: false, ...extra });
const letter = (nav, ch) => press(nav, ch, { code: `Key${ch.toUpperCase()}`, shift: ch !== ch.toLowerCase() });

test("agents are read from the foreground argv, wrappers included", () => {
  assert.equal(agentOf(["npx", "claude"]), "claude");
  assert.equal(agentOf(["/usr/local/bin/codex", "--yolo"]), "codex");
  assert.equal(agentOf(["vim", "notes"]), null);
  assert.equal(agentOf(undefined), null);
});

test("rows nest sessions, panes and children as the sidebar does, without page companions", () => {
  const ws = workspacesOf(tree, branches);
  assert.equal(ws[0].branch, "main");
  assert.equal(ws[1].branch, null, "a remote session has no local branch");
  assert.deepEqual(lines(rows(ws, "", null)), [
    "api",
    "├─server [claude working]",
    "└─review",
    "  ├─codex · review [codex blocked]",
    "  ├─terminal · review [terminal shell]",
    "  └─worker [claude done]",
    "infra",
    "└─shell [terminal shell]",
  ]);
});

test("a split pane without the status reads unknown when an agent runs there", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[1].splitForeground = ["claude"];
  const review = workspacesOf(t, {})[0].panes.filter(p => p.session === "S2");
  assert.deepEqual(review.map(p => p.status), ["blocked", "unknown"]);
});

test("status filters keep their panes and hide empty workspaces", () => {
  const ws = workspacesOf(tree, branches);
  assert.deepEqual(lines(rows(ws, "", "b")), ["api", "└─review", "  └─codex · review [codex blocked]"]);
  assert.deepEqual(lines(rows(ws, "", "d")), ["api", "└─review", "  └─worker [claude done]"]);
  assert.deepEqual(lines(rows(ws, "", "w")), ["api", "└─server [claude working]"]);
});

test("the top-level filter shows one row per top-level session", () => {
  const ws = workspacesOf(tree, branches);
  const list = rows(ws, "", "t");
  assert.deepEqual(lines(list), ["api", "├─server [claude working]", "└─review [codex blocked]", "infra", "└─shell [terminal shell]"]);
  assert.equal(list.reduce((n, r) => n + r.terminals, 0), 4);
});

test("every query word must match one field; a matching workspace keeps all its panes", () => {
  const ws = workspacesOf(tree, branches);
  assert.deepEqual(lines(rows(ws, "docs", null)), ["api", "└─review", "  └─terminal · review [terminal shell]"]);
  assert.deepEqual(lines(rows(ws, "worker claude", null)), []);
  assert.equal(rows(ws, "main", null).filter(r => r.pane).length, 4, "the branch matches the workspace");
  assert.deepEqual(lines(rows(ws, "INFRA", null)), ["infra", "└─shell [terminal shell]"]);
});

test("the window opens on the focused pane and Enter opens it", () => {
  const nav = new Navigator(tree, branches);
  const v = nav.view();
  assert.equal(v.rows[v.selected].target, "p:S2:right");
  assert.equal(v.count, "5 terminals");
  assert.deepEqual(press(nav, "Enter"), { handled: true, open: "p:S2:right" });
});

test("list keys move, jump workspaces and filter", () => {
  const nav = new Navigator(tree, branches);
  letter(nav, "j");
  assert.equal(nav.openTarget(), "p:S3:left");
  letter(nav, "k");
  letter(nav, "k");
  assert.equal(nav.openTarget(), "p:S2:left");
  press(nav, "ArrowRight");
  assert.equal(nav.openTarget(), "p:S5:left");
  press(nav, "ArrowLeft");
  assert.equal(nav.openTarget(), "p:S1:left");
  letter(nav, "G");
  assert.equal(nav.openTarget(), "p:S5:left");
  press(nav, "Home");
  assert.equal(nav.openTarget(), "p:S1:left");

  letter(nav, "w");
  assert.equal(nav.view().search, "/ working");
  press(nav, "Backspace");
  assert.equal(nav.view().placeholder, true);
});

test("the top-level filter moves the selection to its top-level session", () => {
  const nav = new Navigator(tree, branches);
  letter(nav, "j");
  assert.equal(nav.openTarget(), "p:S3:left");
  letter(nav, "t");
  assert.equal(nav.openTarget(), "s:S2");
  letter(nav, "a");
  assert.equal(nav.openTarget(), "s:S2", "dropping `t` keeps the selection");
});

test("search takes the keys until Escape, which then closes", () => {
  const nav = new Navigator(tree, branches);
  press(nav, "/", { code: "Slash" });
  for (const ch of "inf") press(nav, ch);
  assert.equal(nav.view().search, "/ inf");
  assert.equal(nav.openTarget(), "p:S5:left");
  letter(nav, "j");
  assert.equal(nav.view().search, "/ infj", "letters type while searching");
  press(nav, "Backspace");
  assert.deepEqual(press(nav, "Escape"), { handled: true });
  assert.equal(nav.view().searching, false);
  assert.deepEqual(press(nav, "Escape"), { handled: true, close: true });
});

test("chords the navigator does not use pass through", () => {
  const nav = new Navigator(tree, branches);
  assert.deepEqual(press(nav, "w", { code: "KeyW", ctrl: true }), { handled: false });
  assert.equal(press(nav, "d", { code: "KeyD", ctrl: true }).handled, true);
});

test("a filter that leaves nothing shows no rows", () => {
  const nav = new Navigator({ workspaces: [] }, {});
  const v = nav.view();
  assert.deepEqual(v.rows, []);
  assert.equal(v.selected, null);
  assert.deepEqual(press(nav, "Enter"), { handled: true });
});

test("list keys follow the letter the layout types, and the position only without a Latin letter", () => {
  const nav = new Navigator(tree, branches);
  // AZERTY: the key typing `w` sits at KeyZ, the one typing `z` at KeyW
  press(nav, "w", { code: "KeyZ" });
  assert.equal(nav.view().search, "/ working");
  press(nav, "a", { code: "KeyQ" });
  assert.equal(nav.view().placeholder, true);
  press(nav, "z", { code: "KeyW" });
  assert.equal(nav.view().placeholder, true, "z is no filter");
  // Caps Lock: `j` types J without Shift and still moves down; Shift+g is G, the last row
  press(nav, "Home");
  press(nav, "J", { code: "KeyJ" });
  assert.equal(nav.openTarget(), "s:S2");
  press(nav, "G", { code: "KeyG", shift: true });
  assert.equal(nav.openTarget(), "p:S5:left");
  // Cyrillic: the key at KeyJ types о, and still moves down
  press(nav, "Home");
  press(nav, "о", { code: "KeyJ" });
  assert.equal(nav.openTarget(), "s:S2");
});

// The script with a stand-in ngtermctl that serves `tree` and answers `overlay open` with OPEN_STATUS/OPEN_OUT,
// logging every call.
function runScript(args, { treeJson, openStatus = 0, openOut = "" } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "go-to-test-"));
  const fake = path.join(dir, "ngtermctl");
  fs.writeFileSync(fake, `#!/usr/bin/env bash
echo "$*" >>"${dir}/calls"
case "$1 $2 $3" in
  "tree --json "*) cat "${dir}/tree.json" ;;
  "session overlay open") cp "$5" "${dir}/page.html"; printf '%s' "$OPEN_OUT"; exit "$OPEN_STATUS" ;;
esac
`, { mode: 0o755 });
  fs.writeFileSync(path.join(dir, "tree.json"), JSON.stringify({ ok: true, result: { tree: treeJson || tree } }));
  const env = { ...process.env, NGTERMCTL: fake, OPEN_STATUS: String(openStatus), OPEN_OUT: openOut, TMPDIR: dir };
  for (const k of ["AGT_SOCKET", "AGT_WINDOW_ID", "AGT_SESSION_ID", "NGTERM_SOCKET"]) delete env[k];
  const run = spawnSync("bash", [path.join(__dirname, "go-to-navigator.sh"), ...args], { env, encoding: "utf8" });
  const read = f => (fs.existsSync(path.join(dir, f)) ? fs.readFileSync(path.join(dir, f), "utf8") : "");
  const result = { status: run.status, stdout: run.stdout, stderr: run.stderr, calls: read("calls").trim().split("\n"), page: read("page.html") };
  fs.rmSync(dir, { recursive: true, force: true });
  return result;
}

test("outside text cannot open a comment or a tag in the page's data", () => {
  const hostile = structuredClone(tree);
  hostile.workspaces[0].sessions[0].title = "<!--<script>";
  hostile.workspaces[0].name = "</script><b>x";
  const run = runScript([], { treeJson: hostile, openStatus: 2 });
  assert.equal(run.status, 0, run.stderr);
  const data = run.page.split("\n").find(l => l.startsWith("const GOTO = "));
  assert.ok(data && !data.includes("<"), "the data line holds no `<`");
  assert.equal(run.page.match(/<script>/g).length, 3);
  const GOTO = new Function(`${data}; return GOTO;`)();
  assert.equal(GOTO.tree.workspaces[0].sessions[0].title, "<!--<script>");
});

test("a dismissed page does nothing; any other overlay failure keeps its status", () => {
  const dismissed = runScript([], { openStatus: 2 });
  assert.equal(dismissed.status, 0);
  assert.ok(!dismissed.calls.some(c => c.includes(" select ")));
  const refused = runScript([], { openStatus: 1 });
  assert.equal(refused.status, 1);
});

test("a chosen pane is selected and focused; a session only selected", () => {
  const pane = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "p:S2:right" }) });
  assert.equal(pane.status, 0, pane.stderr);
  assert.ok(pane.calls.includes("session select --target S2"));
  assert.ok(pane.calls.includes("session focus right --target S2"));
  const session = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "s:S1" }) });
  assert.ok(session.calls.includes("session select --target S1"));
  assert.ok(!session.calls.some(c => c.startsWith("session focus")));
  const ws = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "w:W2" }) });
  assert.ok(ws.calls.includes("workspace select --target W2"));
});
