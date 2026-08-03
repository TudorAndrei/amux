import { spawn } from "node:child_process"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

const AMUX_BIN = "__AMUX_BIN__"

interface LifecycleEvent {
  reason?: unknown
}

interface LifecycleContext {
  cwd: string
  sessionManager: {
    getSessionId(): string
  }
}

function send(eventName: string, event: LifecycleEvent, ctx: LifecycleContext) {
  return new Promise<void>((resolve) => {
    const child = spawn(AMUX_BIN, ["event", "--agent", "pi", "--event", eventName], {
      stdio: ["pipe", "ignore", "ignore"],
    })
    child.on("close", () => resolve())
    child.on("error", () => resolve())
    child.stdin.end(JSON.stringify({
      type: eventName,
      session_id: ctx.sessionManager.getSessionId(),
      cwd: ctx.cwd,
      ...(typeof event.reason === "string" ? { reason: event.reason } : {}),
    }))
  })
}

export default function amux(pi: ExtensionAPI) {
  pi.on("session_start", async (event, ctx) => {
    await send("session_start", event, ctx)
  })

  pi.on("agent_start", async (event, ctx) => {
    await send("agent_start", event, ctx)
  })

  pi.on("agent_settled", async (event, ctx) => {
    await send("agent_settled", event, ctx)
  })

  pi.on("session_shutdown", async (event, ctx) => {
    await send("session_shutdown", event, ctx)
  })
}
