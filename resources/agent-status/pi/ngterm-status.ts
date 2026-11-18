// ngterm-pi-status-extension
//
// Pi lifecycle extension installed by ngterm's Help ▸ Install Agent Status Hooks… command.
// It uses the installed ngterm wrapper for session, pane, socket, and CLI resolution, so it is
// a harmless no-op outside ngterm.

import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  const wrapper = join(homedir(), ".config", "ngterm", "agent-status", "ngterm-agent-status.sh");
  // Windows has no POSIX shell to run it: Windows PowerShell runs the package's PowerShell form beside it
  const [program, prefix] =
    process.platform === "win32"
      ? [
          "powershell.exe",
          ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", wrapper.replace(/\.sh$/, ".ps1")],
        ]
      : [wrapper, []];

  // Each call starts after the call before it ends, so a slow call cannot put an old status over a new one. On
  // Windows a cold powershell.exe needs 3 to 4 s while the GUI and the agent run, so the call gets 10 s and the
  // handler does not wait for it: the turn continues at once. On other systems the handler waits, as before.
  const windows = process.platform === "win32";
  let last: Promise<void> = Promise.resolve();
  function report(args: string[]): Promise<void> {
    if (!process.env.NGTERM_SESSION_ID) return Promise.resolve();
    // Status reporting is advisory and must never interrupt Pi's agent loop or stop the calls after it.
    last = last
      .then(() => pi.exec(program, [...prefix, ...args], { timeout: windows ? 10_000 : 1_000 }))
      .then(
        () => {},
        () => {},
      );
    return windows ? Promise.resolve() : last;
  }

  pi.on("agent_start", async () => {
    await report(["active", "--blink"]);
  });

  // `agent_settled` waits for automatic retries, compaction retries, and queued continuations.
  pi.on("agent_settled", async () => {
    await report(["completed", "--auto-reset"]);
  });
}
