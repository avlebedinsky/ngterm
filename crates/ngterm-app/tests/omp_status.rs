//! Runs the installed OMP extension under Node's TypeScript loader, without an agent or a live ngterm instance.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const NODE_FLAGS: [&str; 2] = ["--experimental-strip-types", "--disable-warning=ExperimentalWarning"];

const HARNESS: &str = r"import assert from 'node:assert/strict';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const { default: extension } = await import(pathToFileURL(process.env.EXTENSION).href);

// mode: 'ok', 'reject' (the call fails), 'throw' (exec throws before it returns a promise) or 'hold' (each call
// ends only when the test calls finish(), as a slow wrapper does).
function setup(mode = 'ok') {
  const handlers = new Map();
  const calls = [];
  const held = [];
  extension({
    on(event, handler) { handlers.set(event, handler); },
    exec(program, args, options) {
      calls.push({ program, args, options });
      if (mode === 'throw') throw new Error('no such program');
      if (mode === 'reject') return Promise.reject(new Error('wrapper unavailable'));
      const result = { stdout: '', stderr: '', code: 0, killed: false };
      if (mode === 'hold') return new Promise(resolve => held.push(() => resolve(result)));
      return Promise.resolve(result);
    },
  });
  return {
    calls,
    finish() { held.shift()(); },
    emit(type, fields = {}, ctx = undefined) {
      const handler = handlers.get(type);
      return handler ? Promise.resolve(handler({ type, ...fields }, ctx)) : Promise.resolve();
    },
  };
}

// Lets the calls that the handlers did not wait for (on Windows) run.
const drained = () => new Promise(resolve => setTimeout(resolve, 10));

// True when the promise settles within a short time.
function settles(promise) {
  return Promise.race([promise.then(() => true), new Promise(resolve => setTimeout(() => resolve(false), 100))]);
}

for (const platform of ['linux', 'darwin', 'win32']) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
  process.env.NGTERM_SESSION_ID = 'private-test-session';
  const run = setup();
  await run.emit('agent_start');
  await run.emit('agent_end', { willContinue: true });
  await run.emit('agent_end', { willContinue: true, awaitingAsyncWork: true });
  await run.emit('agent_start');
  await run.emit('agent_end', { willContinue: false });
  await run.emit('agent_start');
  await run.emit('agent_end');
  await run.emit('session_shutdown');
  await drained();
  const statuses = [
    ['active', '--blink'], ['active', '--blink'], ['active', '--blink'],
    ['active', '--blink'], ['completed', '--auto-reset'], ['active', '--blink'],
    ['completed', '--auto-reset'], ['idle'],
  ];
  const wrapper = join(homedir(), '.config', 'ngterm', 'agent-status', 'ngterm-agent-status.sh');
  const prefix = platform === 'win32'
    ? ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', wrapper.replace(/\.sh$/, '.ps1')]
    : [];
  assert.deepEqual(run.calls, statuses.map(args => ({
    program: platform === 'win32' ? 'powershell.exe' : wrapper,
    args: [...prefix, ...args], options: { timeout: platform === 'win32' ? 10000 : 1000 },
  })));

  // A task subagent runs this extension again in the same process, with the pane of its parent: only the
  // top-level session reports, so the end and shutdown of a subagent keep the parent active.
  const main = { agent: { kind: 'main', id: 'Main', name: 'main', depth: 0 } };
  const sub = { agent: { kind: 'sub', id: '0-Task', name: 'task', depth: 1, parentId: 'Main' } };
  const clone = { agent: { kind: 'sub', id: '1-Sub', name: 'sub', depth: 0, parentId: 'Main' } };
  const nested = setup();
  await nested.emit('agent_start', {}, main);
  for (const ctx of [sub, clone]) {
    await nested.emit('agent_start', {}, ctx);
    await nested.emit('agent_end', { willContinue: false }, ctx);
    await nested.emit('session_shutdown', {}, ctx);
  }
  await nested.emit('agent_end', { willContinue: false }, main);
  await nested.emit('session_shutdown', {}, main);
  await drained();
  assert.deepEqual(nested.calls.map(call => call.args.slice(prefix.length)), [
    ['active', '--blink'], ['completed', '--auto-reset'], ['idle'],
  ]);

  for (const mode of ['reject', 'throw']) {
    const failing = setup(mode);
    await failing.emit('agent_start');
    await failing.emit('agent_end');
    await failing.emit('session_shutdown');
    await drained();
    assert.deepEqual(failing.calls.map(call => call.args.slice(prefix.length)), [
      ['active', '--blink'], ['completed', '--auto-reset'], ['idle'],
    ], mode);
  }

  // A slow wrapper call: on Windows a cold powershell.exe takes seconds, so the handler does not wait for it and
  // the turn goes on at once; elsewhere the handler waits. On each system a call starts only after the call before
  // it ends, so an old status cannot land over a new one.
  const slow = setup('hold');
  const start = slow.emit('agent_start');
  assert.equal(await settles(start), platform === 'win32', 'agent_start waits on the wrapper call');
  const end = slow.emit('agent_end', { willContinue: false });
  assert.equal(await settles(end), platform === 'win32', 'agent_end waits on the wrapper call');
  assert.deepEqual(slow.calls.map(call => call.args.slice(prefix.length)), [['active', '--blink']]);
  slow.finish();
  assert.equal(await settles(start), true);
  assert.deepEqual(slow.calls.map(call => call.args.slice(prefix.length)), [
    ['active', '--blink'], ['completed', '--auto-reset'],
  ]);
  slow.finish();
  assert.equal(await settles(end), true);

  delete process.env.NGTERM_SESSION_ID;
  const outside = setup();
  await outside.emit('agent_start');
  await outside.emit('agent_end');
  await outside.emit('session_shutdown');
  await drained();
  assert.deepEqual(outside.calls, []);
}
console.log('OMP lifecycle: active, continuations, background work, completed, shutdown, subagents, failures, slow calls, outside ngterm; POSIX and Windows commands');
";

#[test]
fn omp_status_follows_turns_and_never_interrupts_the_agent() -> Result<(), Box<dyn std::error::Error>> {
    let resolved = Command::new("node")
        .args(NODE_FLAGS)
        .args(["-p", "process.execPath"])
        .output();
    let node = match resolved {
        Ok(output) if output.status.success() => PathBuf::from(String::from_utf8(output.stdout)?.trim()),
        result => {
            assert!(
                std::env::var_os("NGTERM_REQUIRE_NODE_TESTS").is_none(),
                "OMP extension tests need Node.js with TypeScript support: {result:?}"
            );
            eprintln!("SKIPPED: OMP extension tests need Node.js with TypeScript support: {result:?}");
            return Ok(());
        }
    };
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("omp-status.mjs");
    fs::write(&script, HARNESS)?;
    let extension = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/omp/ngterm-status.ts");
    let mut command = Command::new(node);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(NODE_FLAGS)
        .arg(script)
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("EXTENSION", extension)
        .output()?;
    assert!(
        output.status.success(),
        "OMP extension failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    Ok(())
}
