import assert from "node:assert/strict"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { execArguments, runTinyboxExec, sandboxCwd } from "../shared/runtime.js"

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

test("captures output and exit status through the shared Node runtime", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "tinybox-adapter-test-"))
  const fake = path.join(directory, "tinybox")
  fs.writeFileSync(fake, "#!/bin/sh\nprintf out\nprintf err >&2\nexit 7\n", { mode: 0o755 })
  try {
    const result = await runTinyboxExec({
      tinybox: fake, task: "task-1-1", cwd: "/workspace", timeout: 50, command: "true",
    })
    assert.deepEqual(result, { exitCode: 7, output: "outerr" })
  } finally {
    fs.rmSync(directory, { recursive: true })
  }
})
