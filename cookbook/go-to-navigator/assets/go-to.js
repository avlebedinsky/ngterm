"use strict";
// The Go to window's model: a window's workspaces as a tree of their sessions' panes, searched by text and
// filtered by agent status. Rows and keys follow ngterm PR #28's navigator, itself after herdr's.
const GoTo = (() => {
  const PLACEHOLDER = "/ search agents and terminals";
  const NO_MATCH = "No matching agents or terminals";
  const HINTS = "↑↓/j/k rows · ←→ workspace · / search · a/b/w/i/d/t filter · enter open · esc close";
  const SEARCH_HINTS = "search type · move ↑↓/ctrl+n/p · open enter · back esc";
  const PAGE = 8;
  const FILTERS = { b: "blocked", w: "working", i: "idle", d: "done", t: "top level" };
  const AGENTS = ["claude", "codex", "opencode", "gemini", "aider", "amp", "pi", "omp", "crush", "goose",
                  "cursor-agent", "copilot", "qwen", "droid", "kiro-cli", "kimi", "auggie"];
  const STATUS = { active: "working", blocked: "blocked", completed: "done" };

  const target = {
    workspace: id => `w:${id}`,
    session: id => `s:${id}`,
    pane: (id, side) => `p:${id}:${side}`,
  };

  function sessionOf(t) {
    const m = /^[sp]:([^:]+)/.exec(t || "");
    return m ? m[1] : null;
  }

  // The agent a pane runs, read from its foreground argv: a wrapper such as `npx claude` still names it.
  function agentOf(argv) {
    if (!Array.isArray(argv)) return null;
    for (const arg of argv) {
      const base = String(arg).split("/").pop();
      if (AGENTS.includes(base)) return base;
    }
    return null;
  }

  function workspacesOf(tree, branches) {
    return ((tree && tree.workspaces) || []).map(ws => {
      const sessions = (ws.sessions || []).filter(s => s.content !== "page");
      const panes = [];
      for (const s of sessions) {
        const split = !!s.hasSplit;
        const holder = split && s.statusPane === "right" ? "right" : "left";
        for (const side of split ? ["left", "right"] : ["left"]) {
          const agent = agentOf(side === "left" ? s.foreground : s.splitForeground);
          let status;
          if (side === holder) status = STATUS[s.status] || (agent ? "idle" : "shell");
          else status = agent ? "unknown" : "shell";
          panes.push({
            session: s.id, parent: s.parent || null, pane: side, sessionName: s.name || "", split,
            // the tree's title, like its cwd and foreground, is the primary pane's
            title: side === "left" ? s.title || null : null,
            cwd: (side === "right" ? s.splitCwd || s.cwd : s.cwd) || "",
            agent, status,
            indicator: side === holder && !!STATUS[s.status],
            current: !!s.active && (!split || (side === "right") === !!s.splitFocused),
          });
        }
      }
      const first = sessions[0];
      const branch = first && !first.remoteHost ? (branches || {})[first.cwd] || null : null;
      return { id: ws.id, name: ws.name || "", branch, panes };
    });
  }

  const kind = p => p.agent || "terminal";
  const paneLabel = p => (p.split ? `${kind(p)} · ${p.sessionName}` : p.sessionName);

  // Sessions nested under their parents, in sidebar order.
  function nodes(ws) {
    const made = new Map();
    for (const p of ws.panes) {
      if (!made.has(p.session)) made.set(p.session, { main: p, panes: [], children: [] });
      made.get(p.session).panes.push(p);
    }
    const roots = [];
    for (const node of made.values()) {
      const parent = node.main.parent && made.get(node.main.parent);
      (parent ? parent.children : roots).push(node);
    }
    return roots;
  }

  function kept(node, keeps) {
    const panes = node.panes.filter(keeps);
    const children = node.children.map(c => kept(c, keeps)).filter(Boolean);
    return panes.length || children.length ? { main: node.main, panes, children } : null;
  }
  const holds = (node, session) => node.main.session === session || node.children.some(c => holds(c, session));
  const nodeCurrent = node => node.panes.some(p => p.current) || node.children.some(nodeCurrent);
  const branchLine = (rails, last) => rails.concat([last ? "└─" : "├─"]);
  const columns = p => ({ agent: kind(p), status: p.status, indicator: p.indicator, current: p.current });

  function workspaceRow(ws) {
    return { target: target.workspace(ws.id), workspace: true, tree: [], label: ws.name, branch: ws.branch,
             detail: ws.name, meta: ws.branch || "", pane: null, terminals: 0 };
  }
  function sessionRow(ws, main, tree) {
    return { target: target.session(main.session), workspace: false, tree, label: main.sessionName,
             detail: `${ws.name} / ${main.sessionName}`, meta: main.cwd, pane: null, terminals: 0 };
  }
  function paneRow(ws, p, tree) {
    const session = `${ws.name} / ${p.sessionName}`;
    return { target: target.pane(p.session, p.pane), workspace: false, tree, label: paneLabel(p),
             detail: p.split ? `${session} / ${kind(p)}` : session, meta: p.cwd, pane: columns(p), terminals: 1 };
  }

  function sessionRows(rows, ws, node, rails, last) {
    const splitPanes = node.main.split ? node.panes : [];
    if (node.panes.length && !node.main.split) rows.push(paneRow(ws, node.panes[0], branchLine(rails, last)));
    else rows.push(sessionRow(ws, node.main, branchLine(rails, last)));
    const inner = rails.concat([last ? "  " : "│ "]);
    const under = splitPanes.length + node.children.length;
    splitPanes.forEach((p, i) => rows.push(paneRow(ws, p, branchLine(inner, i + 1 === under))));
    node.children.forEach((c, i) => sessionRows(rows, ws, c, inner, splitPanes.length + i + 1 === under));
  }

  function topLevelRow(ws, node, tree) {
    let row, holder;
    if (node.main.split) {
      holder = node.panes.find(p => p.indicator) || node.main;
      row = { ...sessionRow(ws, node.main, tree), terminals: node.panes.length };
    } else {
      holder = node.main;
      row = paneRow(ws, node.main, tree);
    }
    return { ...row, pane: { ...columns(holder), current: nodeCurrent(node) } };
  }

  function paneMatches(p, matches) {
    return matches(p.sessionName) || matches(paneLabel(p)) || matches(p.cwd) ||
      (!!p.agent && matches(p.agent)) || (!!p.title && matches(p.title));
  }

  // Every query word must be in one field; a status filter keeps its panes; `t` keeps top-level sessions.
  function rows(workspaces, query, filter) {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const matches = v => {
      const s = String(v).toLowerCase();
      return words.every(w => s.includes(w));
    };
    const status = filter && filter !== "t" ? FILTERS[filter] : null;
    const narrowing = !!status || words.length > 0;
    const out = [];
    for (const ws of workspaces) {
      const wsMatches = matches(ws.name) || (!!ws.branch && matches(ws.branch));
      const keeps = p => (!status || p.status === status) && (wsMatches || paneMatches(p, matches));
      const sessions = nodes(ws).map(n => kept(n, keeps)).filter(Boolean);
      if (!narrowing || sessions.length || (!status && wsMatches)) {
        out.push(workspaceRow(ws));
        sessions.forEach((n, i) => {
          const last = i === sessions.length - 1;
          if (filter === "t") out.push(topLevelRow(ws, n, branchLine([], last)));
          else sessionRows(out, ws, n, [], last);
        });
      }
    }
    return out;
  }

  // The selected row: its target's, else its session's, else its session's main pane; with none, the first pane row.
  function selectedIndex(list, selected) {
    if (!list.length) return null;
    const pos = t => {
      const i = list.findIndex(r => r.target === t);
      return i < 0 ? null : i;
    };
    if (selected) {
      const session = sessionOf(selected);
      if (session) return pos(selected) ?? pos(target.session(session)) ?? pos(target.pane(session, "left")) ?? 0;
      return pos(selected) ?? 0;
    }
    const i = list.findIndex(r => r.pane);
    return i < 0 ? 0 : i;
  }

  function topLevelTarget(workspaces, t) {
    const session = sessionOf(t);
    if (!session) return t;
    for (const ws of workspaces) for (const n of nodes(ws)) if (holds(n, session)) return target.session(n.main.session);
    return t;
  }

  // A list key is the letter it types, or, on a layout that types no Latin letter, the one at its position.
  function letterOf(ev) {
    // the case comes from Shift, not from what Caps Lock makes the key type
    if (/^[A-Za-z]$/.test(ev.key || "")) return ev.shift ? ev.key.toUpperCase() : ev.key.toLowerCase();
    if (/^Key[A-Z]$/.test(ev.code || "")) return ev.shift ? ev.code.slice(3) : ev.code.slice(3).toLowerCase();
    if (ev.code === "Slash" && !ev.shift) return "/";
    return ev.key;
  }

  class Navigator {
    constructor(tree, branches) {
      this.branches = branches || {};
      this.query = "";
      this.searching = false;
      this.filter = null;
      this.setTree(tree);
      this.selected = this.currentTarget();
    }

    setTree(tree) {
      this.workspaces = workspacesOf(tree, this.branches);
    }

    currentTarget() {
      for (const ws of this.workspaces) for (const p of ws.panes) if (p.current) return target.pane(p.session, p.pane);
      return null;
    }

    view() {
      const list = rows(this.workspaces, this.query, this.filter);
      const selected = selectedIndex(list, this.selected);
      let search;
      if (this.filter && !this.searching) search = `/ ${FILTERS[this.filter]}`;
      else if (this.searching || this.query) search = `/ ${this.query}`;
      else search = PLACEHOLDER;
      const count = list.reduce((n, r) => n + r.terminals, 0);
      return {
        rows: list, selected, search, searching: this.searching,
        placeholder: search === PLACEHOLDER && !this.searching,
        count: `${count} ${count === 1 ? "terminal" : "terminals"}`,
        hints: this.searching ? SEARCH_HINTS : HINTS,
        empty: NO_MATCH,
      };
    }

    step(delta) {
      const list = rows(this.workspaces, this.query, this.filter);
      const i = selectedIndex(list, this.selected);
      if (i === null) {
        this.selected = null;
        return;
      }
      this.selected = list[Math.max(0, Math.min(list.length - 1, i + delta))].target;
    }

    selectLast() {
      const list = rows(this.workspaces, this.query, this.filter);
      this.selected = list.length ? list[list.length - 1].target : null;
    }

    jumpWorkspace(forward) {
      const list = rows(this.workspaces, this.query, this.filter);
      const sel = selectedIndex(list, this.selected);
      if (sel === null) return;
      let section = sel;
      for (let i = sel; i >= 0; i--) {
        if (list[i].workspace) {
          section = i;
          break;
        }
      }
      const heads = [];
      for (let i = 0; i + 1 < list.length; i++) {
        if (list[i].workspace && !list[i + 1].workspace && (forward ? i > section : i < section)) heads.push(i);
      }
      const head = forward ? heads[0] : heads[heads.length - 1];
      if (head === undefined) return;
      const row = list.slice(head).find(r => r.pane);
      if (row) this.selected = row.target;
    }

    dropFilter() {
      const was = this.filter;
      this.filter = null;
      if (was !== "t") this.selected = null;
    }

    queryChanged() {
      this.filter = null;
      this.selected = null;
    }

    select(t) {
      this.selected = t;
    }

    openTarget() {
      const list = rows(this.workspaces, this.query, this.filter);
      const i = selectedIndex(list, this.selected);
      return i === null ? null : list[i].target;
    }

    // ev: {key, code, shift, ctrl}. Returns {handled}, plus {close: true} or {open: target}.
    key(ev) {
      const ctrl = ev.ctrl ? String(letterOf(ev)).toLowerCase() : null;
      if (ev.ctrl && !["n", "p", "u", "d"].includes(ctrl)) return { handled: false };
      if (ev.key === "Escape") {
        if (!this.searching) return { handled: true, close: true };
        this.searching = false;
        return { handled: true };
      }
      if (ev.key === "Enter") {
        const t = this.openTarget();
        return t ? { handled: true, open: t } : { handled: true };
      }
      return { handled: this.searching ? this.searchKey(ev, ctrl) : this.listKey(ev, ctrl) };
    }

    searchKey(ev, ctrl) {
      if (ev.key === "ArrowUp" || ctrl === "p") this.step(-1);
      else if (ev.key === "ArrowDown" || ctrl === "n") this.step(1);
      else if (ev.key === "Backspace") {
        if (this.query) {
          this.query = [...this.query].slice(0, -1).join("");
          this.queryChanged();
        }
      } else if (!ev.ctrl && [...ev.key].length === 1) {
        this.query += ev.key;
        this.queryChanged();
      } else return false;
      return true;
    }

    listKey(ev, ctrl) {
      const k = ev.ctrl ? null : letterOf(ev);
      if (ev.key === "ArrowUp" || k === "k") this.step(-1);
      else if (ev.key === "ArrowDown" || k === "j") this.step(1);
      else if (ctrl === "u" || ev.key === "PageUp") this.step(-PAGE);
      else if (ctrl === "d" || ev.key === "PageDown") this.step(PAGE);
      else if (ev.key === "ArrowLeft") this.jumpWorkspace(false);
      else if (ev.key === "ArrowRight") this.jumpWorkspace(true);
      else if (ev.key === "Home") this.selected = null;
      else if (ev.key === "End" || k === "G") this.selectLast();
      else if (ev.key === "Backspace") {
        if (this.filter) this.dropFilter();
      } else if (k === "/") {
        this.searching = true;
        this.filter = null;
      } else if (k === "a") {
        this.query = "";
        this.dropFilter();
      } else if (k && Object.hasOwn(FILTERS, k)) {
        this.query = "";
        this.filter = k;
        this.selected = k === "t" && this.selected ? topLevelTarget(this.workspaces, this.selected) : null;
      } else return false;
      return true;
    }
  }

  return { Navigator, agentOf, workspacesOf, rows, selectedIndex, target };
})();

if (typeof module !== "undefined") module.exports = GoTo;
