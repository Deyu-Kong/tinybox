import { tool } from "@opencode-ai/plugin"
import { execArguments, sandboxCwd } from "./runtime.js"

export default tool({
  description: "Run a shell command in the persistent tinybox task",
  args: {
    command: tool.schema.string(),
    timeout: tool.schema.number().int().positive().max(3600000).optional(),
    workdir: tool.schema.string().optional(),
    description: tool.schema.string().optional(),
  },
  async execute(args, context) {
    const task = process.env.TINYBOX_TASK_ID
    if (!task) throw new Error("TINYBOX_TASK_ID is required; refusing host fallback")
    const tinybox = process.env.TINYBOX_BIN || "tinybox"
    const cwd = sandboxCwd(context.worktree, args.workdir || context.worktree)
    const timeout = args.timeout || 120000
    const proc = Bun.spawn(
      execArguments(tinybox, task, cwd, timeout, args.command),
      { stdout: "pipe", stderr: "pipe", env: process.env },
    )
    const abort = () => {
      proc.kill()
      Bun.spawnSync([tinybox, "agent", "destroy", task], { stdout: "ignore", stderr: "ignore", env: process.env })
    }
    context.abort.addEventListener("abort", abort, { once: true })
    try {
      const [stdout, stderr, exitCode] = await Promise.all([
        new Response(proc.stdout).text(),
        new Response(proc.stderr).text(),
        proc.exited,
      ])
      const output = `${stdout}${stderr}`
      if (exitCode !== 0) throw new Error(`tinybox command exited ${exitCode}\n${output}`)
      return output
    } finally {
      context.abort.removeEventListener("abort", abort)
    }
  },
})
