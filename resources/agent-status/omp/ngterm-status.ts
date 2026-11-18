// ngterm-omp-status-extension

import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

export default function (omp: ExtensionAPI) {
  const wrapper = join(homedir(), ".config", "ngterm", "agent-status", "ngterm-agent-status.sh");
  const [program, prefix]: [string, string[]] =
    process.platform === "win32"
      ? [
          "powershell.exe",
          ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", wrapper.replace(/\.sh$/, ".ps1")],
        ]
      : [wrapper, []];

  // OMP binds this extension again to each subagent session (task tool, eval agent(), /tan clones), in the same
  // process and with the same NGTERM_SESSION_ID. Only the top-level session owns the status of the pane: a subagent
  // that ends or shuts down while the parent works must not show done or clear the status. An OMP without
  // ctx.agent has no subagent identity, so the extension reports for each session there.
  //
  // Each call starts after the call before it ends, so a slow call cannot put an old status over a new one. On
  // Windows a cold powershell.exe needs 3 to 4 s while the GUI and the agent run, so the call gets 10 s and the
  // handler does not wait for it: the turn continues at once. On other systems the wrapper ends in much less than
  // 1 s, and the handler waits for it.
  const windows = process.platform === "win32";
  let last: Promise<void> = Promise.resolve();
  function report(ctx: ExtensionContext | undefined, args: string[]): Promise<void> {
    if (!process.env.NGTERM_SESSION_ID || ctx?.agent?.kind === "sub") return Promise.resolve();
    // A status failure must not stop the agent or the calls after it.
    last = last
      .then(() => omp.exec(program, [...prefix, ...args], { timeout: windows ? 10_000 : 1_000 }))
      .then(
        () => {},
        () => {},
      );
    return windows ? Promise.resolve() : last;
  }

  omp.on("agent_start", async (_event, ctx) => {
    await report(ctx, ["active", "--blink"]);
  });

  omp.on("agent_end", async (event, ctx) => {
    // Retries, queued continuations and background work have not finished the turn.
    await report(ctx, event.willContinue ? ["active", "--blink"] : ["completed", "--auto-reset"]);
  });

  omp.on("session_shutdown", async (_event, ctx) => {
    await report(ctx, ["idle"]);
  });
}
