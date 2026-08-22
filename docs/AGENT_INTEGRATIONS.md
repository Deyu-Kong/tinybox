# Agent integration status

> Evidence date: 2026-08-22. These statuses describe the versions and commands
> actually exercised below, not blanket compatibility promises.

## Support matrix

| Agent | Integration | Status | Evidence and boundary |
|---|---|---|---|
| OpenCode 1.18.18 | Host Agent + user-level custom `bash` override → persistent task exec | **experimental** | `agent integrate` installs once; `agent launch` forces the adapter through `OPENCODE_CONFIG_DIR`, creates one task, and destroys it on exit. Contract, config-load, and recorded live LLM tool calls pass. |
| Codex CLI 0.148.0 | Whole Agent process in `tinybox agent run --profile node` | **experimental smoke only** | `codex --version` runs inside tinybox under root acceptance. Buffered execution has no controlling TTY; config/auth injection, interactive UI, `resume`, and nested Codex sandbox behavior are therefore **unsupported** in this MVP. tinybox does not claim to replace Codex's built-in shell sandbox. |
| Pi coding agent 0.73.0 | Host Agent + user-level extension overriding `bash` → persistent task exec | **experimental** | Uses the official same-name tool override and explicit `--extension` loading. Shared runtime, install contract, real Alibaba Model Studio request, tinybox Bash tool call, and task cleanup all pass. |
| Generic CLI | Whole process or detached task + `agent exec` | **supported (experimental runtime)** | Root acceptance covers foreground exit/output/cleanup and detached repeated exec/stop/destroy. No TTY streaming or attach. |

## OpenCode

OpenCode supports global tools under `~/.config/opencode/tools/` and gives same-name custom
tools precedence over built-ins. tinybox installs once and launches OpenCode with that
directory as the explicit highest-priority custom directory:

```bash
tinybox agent integrate opencode
tinybox agent launch opencode /path/to/project
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

Pi supports user-level extensions under `~/.pi/agent/extensions/` and same-name built-in
tool replacement. tinybox installs an extension once and loads its exact path at launch:

```bash
tinybox agent integrate pi
tinybox agent launch pi /path/to/project
```

It shares the OpenCode adapter's argv-safe task exec implementation and forwards command,
cwd, timeout, cancellation, output, and exit status. Pi 0.73.0 completed a live model tool
call that emitted `PI_TINYBOX_LIVE_OK` from the task, followed by successful task cleanup.

Source: [Pi extensions documentation](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md).
