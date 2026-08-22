# Local installation and operation

tinybox supports Linux kernel 5.10+, cgroup v2, OverlayFS, and Landlock. It is
rootful and shares the host kernel. Install it only on a personal machine,
dedicated Agent runner, or tenant-isolated VM—not as a hostile multi-tenant boundary.

## Install

From a trusted checkout:

```bash
./scripts/install.sh
```

This builds with Cargo and installs to the invoking user's `~/.local` by default.
Set `TINYBOX_INSTALL_PREFIX` for another writable prefix. It installs the runtime,
OpenCode wrapper/installer, and adapter sources; it does not install a daemon service.

Verify the host before starting the daemon:

```bash
sudo ~/.local/bin/tinybox doctor
sudo ~/.local/bin/tinybox doctor --json
```

## Temporary daemon and first command

Terminal 1:

```bash
sudo ~/.local/bin/tinybox daemon --listen 127.0.0.1:8080
```

Terminal 2 (the CLI may remain the normal desktop user; only the daemon is root):

```bash
~/.local/bin/tinybox agent run --profile rust . -- cargo test
```

The foreground command creates and destroys its task automatically. For an OpenCode
session:

```bash
~/.local/bin/tinybox-install-opencode-adapter .
TINYBOX_BIN=~/.local/bin/tinybox ~/.local/bin/tinybox-opencode .
```

OpenCode support is experimental and replaces only its `bash` tool. Review
[AGENT_INTEGRATIONS.md](AGENT_INTEGRATIONS.md) before use.

## Service choice

MVP documentation intentionally recommends a foreground temporary daemon. A system-wide
systemd unit is not shipped because the HTTP daemon has no authentication and must remain
bound to loopback. If an administrator creates a unit, use `127.0.0.1`, restrict local
access, and treat daemon restart as task loss; only empty orphan state is reconciled.

## Cleanup and uninstall

List and explicitly destroy detached tasks before stopping the daemon:

```bash
~/.local/bin/tinybox agent list
~/.local/bin/tinybox agent destroy TASK_ID
```

Then run `./scripts/uninstall.sh` from the checkout. It removes only the known files it
installed and retains parent directories. Project-local `.opencode/tools/bash.ts` and
`runtime.js` are user project files and are deliberately not deleted automatically.

## Troubleshooting

- `doctor` reports missing controllers: tinybox requires cgroup v2 with cpu, memory, and
  pids controllers delegated at `/sys/fs/cgroup`.
- `Landlock ABI is unavailable`: use a supported kernel with Landlock enabled; policy/task
  execution fails closed.
- `unable to allocate pty` under WSL2 sudo: repair `/dev/pts` with
  `sudo mount -t devpts devpts /dev/pts`.
- daemon connection errors never invoke the requested command on the host. Start the daemon
  or pass the correct `--daemon` address.
- interactive Codex/OpenCode TTY streaming and attach are not available in this MVP.
- after daemon `SIGKILL`, start it once on the same host to reconcile empty orphan task
  cgroups/state; active cgroups owned by another daemon are not touched.
