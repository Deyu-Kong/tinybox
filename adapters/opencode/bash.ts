import { tool } from "@opencode-ai/plugin"
import { runTinyboxExec, sandboxCwd } from "./runtime.js"

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
    const result = await runTinyboxExec({
      tinybox, task, cwd, timeout, command: args.command, signal: context.abort,
    })
    if (result.exitCode !== 0) {
      throw new Error(`tinybox command exited ${result.exitCode}\n${result.output}`)
    }
    return result.output
  },
})
