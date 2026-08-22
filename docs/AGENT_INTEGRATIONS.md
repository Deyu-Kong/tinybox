# Agent integration status

> Evidence date: 2026-08-22. These statuses describe the versions and commands
> actually exercised below, not blanket compatibility promises.

## Support matrix

| Agent | Integration | Status | Evidence and boundary |
|---|---|---|---|
| OpenCode 1.18.18 | Host Agent + project custom `bash` tool → persistent task exec | **experimental** | Official custom-tool discovery and same-name override are used. `runtime.test.mjs` verifies cwd/argv/fail-closed; `opencode debug config` loads the installed tool. A live LLM tool call is left to the demo because it requires the user's provider credentials. |
| Codex CLI 0.148.0 | Whole Agent process in `tinybox agent run --profile node` | **experimental smoke only** | `codex --version` runs inside tinybox under root acceptance. Buffered execution has no controlling TTY; config/auth injection, interactive UI, `resume`, and nested Codex sandbox behavior are therefore **unsupported** in this MVP. tinybox does not claim to replace Codex's built-in shell sandbox. |
| Pi coding agent | TypeScript extension registering `tinybox_bash` | **unsupported / spike** | The extension follows the official `pi.registerTool()` interface, but no Pi binary is installed in the acceptance environment, so it is not promoted to experimental support. |
| Generic CLI | Whole process or detached task + `agent exec` | **supported (experimental runtime)** | Root acceptance covers foreground exit/output/cleanup and detached repeated exec/stop/destroy. No TTY streaming or attach. |

## OpenCode

OpenCode documents project tools under `.opencode/tools/`, and states that a custom
tool with the same name as a built-in tool takes precedence. tinybox uses that mechanism
to replace `bash` without modifying OpenCode core:

```bash
scripts/install_opencode_adapter.sh /path/to/project
TINYBOX_BIN="$PWD/target/debug/tinybox" scripts/tinybox-opencode /path/to/project
```

The wrapper creates one detached task for the OpenCode session. Every `bash` call becomes
a short `tinybox agent exec`; workspace-relative cwd and timeout are forwarded. Host paths
outside the workspace are rejected. Missing `TINYBOX_TASK_ID`, daemon errors, timeout, and
nonzero exit never fall back to host shell. Cancellation destroys the task because the MVP
has no keep-task per-exec cancel endpoint.

Source: [OpenCode custom tools](https://opencode.ai/docs/custom-tools/).

## Codex

The only verified command is deliberately small:

```bash
tinybox agent run --profile node . -- codex --version
```

The Codex CLI supports `--cd`, `resume`, sandbox selection, and noninteractive `exec`, but
tinybox's current buffered HTTP exec does not provide a controlling terminal and private
HOME does not import `~/.codex` credentials or sessions. A real wrapper would need explicit
read-only auth/config injection, private writable session state, streaming stdio, terminal
resize, and nested-sandbox tests. Until those exist, interactive Codex is unsupported.

Source: [official OpenAI Codex CLI reference](https://developers.openai.com/codex/cli/reference/).

## Pi

`adapters/pi/tinybox.ts` is a source-level spike using `pi.registerTool()`. It forwards
command and timeout, returns output plus exit status, and destroys the task on cancellation.
Install/runtime smoke is required before changing the status.

Source: [Pi extensions documentation](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md).
