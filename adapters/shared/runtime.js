import path from "node:path"
import { spawn, spawnSync } from "node:child_process"

export function sandboxCwd(worktree, requested) {
  const relative = path.relative(worktree, path.resolve(requested || worktree))
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("workdir must stay inside the Agent workspace")
  }
  return relative ? `/workspace/${relative}` : "/workspace"
}

export function execArguments(tinybox, task, cwd, timeout, command) {
  if (!task) throw new Error("TINYBOX_TASK_ID is required; refusing host fallback")
  return [tinybox, "agent", "exec", task, "--cwd", cwd, "--timeout-ms", String(timeout),
    "--", "/bin/sh", "-lc", command]
}

export function runTinyboxExec({ tinybox, task, cwd, timeout, command, signal }) {
  const argv = execArguments(tinybox, task, cwd, timeout, command)
  return new Promise((resolve, reject) => {
    const child = spawn(argv[0], argv.slice(1), {
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    const stdout = []
    const stderr = []
    child.stdout.on("data", chunk => stdout.push(chunk))
    child.stderr.on("data", chunk => stderr.push(chunk))
    const abort = () => {
      child.kill()
      spawnSync(tinybox, ["agent", "destroy", task], { stdio: "ignore", env: process.env })
    }
    signal?.addEventListener("abort", abort, { once: true })
    child.once("error", reject)
    child.once("close", code => {
      signal?.removeEventListener("abort", abort)
      resolve({
        exitCode: code ?? 125,
        output: Buffer.concat([...stdout, ...stderr]).toString(),
      })
    })
  })
}
