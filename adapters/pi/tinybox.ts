import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"
import { Type } from "typebox"

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "tinybox_bash",
    label: "tinybox bash",
    description: "Run a command in the persistent tinybox task",
    parameters: Type.Object({
      command: Type.String(),
      timeoutMs: Type.Optional(Type.Number({ minimum: 1, maximum: 3600000 })),
    }),
    async execute(_id, params, signal) {
      const task = process.env.TINYBOX_TASK_ID
      if (!task) throw new Error("TINYBOX_TASK_ID is required; refusing host fallback")
      const tinybox = process.env.TINYBOX_BIN || "tinybox"
      const child = Bun.spawn([
        tinybox, "agent", "exec", task, "--timeout-ms", String(params.timeoutMs || 120000),
        "--", "/bin/sh", "-lc", params.command,
      ], { stdout: "pipe", stderr: "pipe", env: process.env })
      const abort = () => {
        child.kill()
        Bun.spawnSync([tinybox, "agent", "destroy", task], { stdout: "ignore", stderr: "ignore", env: process.env })
      }
      signal.addEventListener("abort", abort, { once: true })
      try {
        const [stdout, stderr, code] = await Promise.all([
          new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ])
        return {
          content: [{ type: "text", text: `${stdout}${stderr}` }],
          details: { exitCode: code },
          isError: code !== 0,
        }
      } finally {
        signal.removeEventListener("abort", abort)
      }
    },
  })
}
