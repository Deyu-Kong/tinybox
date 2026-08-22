import path from "node:path"

export function sandboxCwd(worktree, requested) {
  const relative = path.relative(worktree, path.resolve(requested || worktree))
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("workdir must stay inside the OpenCode workspace")
  }
  return relative ? `/workspace/${relative}` : "/workspace"
}

export function execArguments(tinybox, task, cwd, timeout, command) {
  if (!task) throw new Error("TINYBOX_TASK_ID is required; refusing host fallback")
  return [tinybox, "agent", "exec", task, "--cwd", cwd, "--timeout-ms", String(timeout), "--", "/bin/sh", "-lc", command]
}
