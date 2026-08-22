import type { ExtensionAPI } from "@mariozechner/pi-coding-agent"
import { Type } from "@sinclair/typebox"
import { runTinyboxExec, sandboxCwd } from "./runtime.js"

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "bash",
    label: "bash (tinybox)",
    description: "Run a command in the persistent tinybox task",
    parameters: Type.Object({
      command: Type.String(),
      timeoutMs: Type.Optional(Type.Number({ minimum: 1, maximum: 3600000 })),
    }),
    async execute(_id, params, signal, _onUpdate, context) {
      const task = process.env.TINYBOX_TASK_ID
      if (!task) throw new Error("TINYBOX_TASK_ID is required; refusing host fallback")
      const tinybox = process.env.TINYBOX_BIN || "tinybox"
      const cwd = sandboxCwd(context.cwd, context.cwd)
      const result = await runTinyboxExec({
        tinybox, task, cwd, timeout: params.timeoutMs || 120000,
        command: params.command, signal,
      })
      return {
        content: [{ type: "text", text: result.output }],
        details: { exitCode: result.exitCode },
        isError: result.exitCode !== 0,
      }
    },
  })
}
