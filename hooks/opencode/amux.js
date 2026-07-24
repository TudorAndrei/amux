import { spawn } from "node:child_process"

const AMUX_BIN = "__AMUX_BIN__"

function send(payload, args = []) {
  return new Promise((resolve) => {
    const child = spawn(AMUX_BIN, ["event", "--agent", "opencode", ...args], {
      stdio: ["pipe", "ignore", "ignore"],
    })
    child.on("close", resolve)
    child.on("error", resolve)
    child.stdin.end(JSON.stringify(payload))
  })
}

export const AmuxPlugin = async (ctx) => {
  return {
    event: async ({ event }) => {
      const type = event?.type || "event"
      const attentionArgs = type === "session.idle"
        ? ["--attention", "1", "--reason", "session idle"]
        : []
      await send({
        event,
        directory: ctx.directory,
        worktree: ctx.worktree,
      }, ["--event", type, ...attentionArgs])
    },
  }
}

export default AmuxPlugin
