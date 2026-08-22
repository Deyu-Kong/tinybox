import assert from "node:assert/strict"
import test from "node:test"
import { execArguments, sandboxCwd } from "./runtime.js"

test("maps only workspace-contained working directories", () => {
  assert.equal(sandboxCwd("/project", "/project"), "/workspace")
  assert.equal(sandboxCwd("/project", "/project/crate"), "/workspace/crate")
  assert.throws(() => sandboxCwd("/project", "/secret"), /must stay inside/)
})

test("builds an argv-safe tinybox exec without a shell on the host", () => {
  assert.deepEqual(execArguments("tinybox", "task-1-1", "/workspace", 50, "echo $HOME"), [
    "tinybox", "agent", "exec", "task-1-1", "--cwd", "/workspace", "--timeout-ms", "50",
    "--", "/bin/sh", "-lc", "echo $HOME",
  ])
  assert.throws(() => execArguments("tinybox", "", "/workspace", 50, "true"), /refusing host fallback/)
})
