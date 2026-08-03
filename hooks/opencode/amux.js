import { spawn } from "node:child_process"

const AMUX_BIN = "__AMUX_BIN__"
const LIFECYCLE_EVENTS = new Set([
  "session.created",
  "session.status",
  "permission.asked",
  "permission.replied",
  "session.deleted",
])

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
      if (!LIFECYCLE_EVENTS.has(type)) return
      const sessionID = event?.properties?.sessionID
      const statusType = event?.properties?.status?.type
      const properties = {
        ...(typeof sessionID === "string" ? { sessionID } : {}),
        ...(type === "session.status" && typeof statusType === "string"
          ? { status: { type: statusType } }
          : {}),
      }
      await send({
        event: { type, properties },
        directory: ctx.directory,
        worktree: ctx.worktree,
      }, ["--event", type])
    },
  }
}

export default AmuxPlugin
