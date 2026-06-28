import { spawn } from "node:child_process"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

const AMUX_BIN = "__AMUX_BIN__"

function send(eventName: string, event: unknown, ctx: unknown) {
  return new Promise<void>((resolve) => {
    const child = spawn(AMUX_BIN, ["event", "--agent", "pi", "--event", eventName], {
      stdio: ["pipe", "ignore", "ignore"],
    })
    child.on("close", () => resolve())
    child.on("error", () => resolve())
    child.stdin.end(JSON.stringify({
      type: eventName,
      event,
      ctx,
    }))
  })
}

export default function amux(pi: ExtensionAPI) {
  pi.on("session_start", async (event, ctx) => {
    await send("session_start", event, ctx)
  })

  pi.on("tool_call", async (event, ctx) => {
    await send("tool_call", event, ctx)
  })

  pi.on("tool_result", async (event, ctx) => {
    await send("tool_result", event, ctx)
  })
}

