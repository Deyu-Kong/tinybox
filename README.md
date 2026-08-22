# tinybox — Lightweight Local Containers for Coding Agents

A small, self-hosted Linux container runtime intended to integrate with
OpenCode, Pi Agent, Codex, and other CLI coding agents. tinybox combines a
focused subset of Docker-style Linux isolation with E2B-style persistent
sandbox sessions, repeated exec, and environment lifecycle APIs.

![Phase Progress](https://img.shields.io/badge/phases-1--13-audited-yellow)
![Lines of Code](https://img.shields.io/badge/Rust_LOC-2306-orange)
![License](https://img.shields.io/badge/license-MIT-green)
![Status](https://img.shields.io/badge/status-experimental%2C%20open%20correctness%20gaps-orange)

> ⚠️ **Experimental, rootful runtime; not a production security boundary.**
> The original M1 P0 findings and the later C0–C3 correctness gaps have been
> addressed and regression-tested. The runtime remains experimental because
> daemon authentication/persistence, rootless operation, full OCI semantics,
> and a hostile-tenant kernel boundary are not provided. See
> [docs/PLAN.md](docs/PLAN.md) for the authoritative status.

tinybox is aimed at **individual Linux developer machines, dedicated Agent
runners, and tenant-isolated VMs**. It is deliberately a feature subset, not a
Docker replacement or hosted E2B competitor. It does not provide an independent
guest kernel, so it is not an alternative to a VM/microVM for hostile cloud
tenants.

## Why tinybox

Local coding agents are often either allowed to run directly on the developer's
machine or placed inside a general container/remote sandbox that requires image,
environment, and lifecycle setup. tinybox targets the gap between them:

- local-first and self-hosted;
- no Docker daemon, containerd, or runc dependency;
- no project-specific Dockerfile in the intended host-environment mode;
- one persistent task per Agent session, with repeated isolated tool exec;
- host/rootfs/profile environment modes without a full image platform;
- task-private rootfs, home, caches, and volumes;
- a deliberately small API rather than a general container platform.

“Lightweight” currently means a smaller product surface and shorter local setup
path. Startup, memory, and disk-cost claims remain to be measured by
the MVP benchmark before they are advertised as performance advantages.

The intended integration keeps the Agent UX unchanged:

```text
OpenCode / Pi / Codex / CLI Agent (target integrations)
  ├── Agent conversation and approval
  └── shell / compiler / package manager / tests
                    │ stdout/stderr/exit code
                    ▼
              tinybox adapter
                    ▼
       persistent task + repeated exec
       ├── workspace
       ├── rootfs + private home/cache/volumes
       ├── namespaces + cgroup + seccomp
       └── deterministic cleanup and destroy
```

Agent-native task/exec, environment APIs, and the Agent lifecycle CLI have passed
their M0–M2 root acceptance gates. Agent-specific adapters are still implementation
targets, so this is not yet a released MVP. See the active
[MVP plan](docs/PRODUCT_PLAN.md), [product vision](docs/VISION.md), and
[OpenCode demo design](docs/OPENCODE_DEMO.md).

## Relationship to Docker and Agent Sandboxes

The underlying isolation and snapshot primitives are not novel. tinybox's goal
is to package a useful subset for personal, local Agent workflows.

| Product | Execution boundary and delivery | Best fit | Difference from tinybox |
|---|---|---|---|
| [Docker](https://docs.docker.com/) / Docker Sandboxes | General container ecosystem plus Agent-oriented VM products | Full container workflows and stronger packaged environments | tinybox intentionally omits build, Compose, orchestration, and most image-platform features; anything tinybox does can generally be assembled with Docker and scripts. |
| [E2B](https://www.e2b.dev/docs) | Managed Agent sandboxes with templates, SDKs, persistence, and VM isolation | Cloud Agent execution and tenant isolation | tinybox is local, self-hosted, same-kernel, and much smaller in scope; it is not an E2B security equivalent. |
| [Anthropic Sandbox Runtime](https://github.com/anthropic-experimental/sandbox-runtime) | Lightweight local wrapper; bubblewrap on Linux plus host network proxies | Local command and MCP sandboxing | This is the closest product shape. tinybox explores a Rust runtime with cgroup ceilings, structured task audit, and its own no-NIC broker, but is less mature and Linux-only. |
| [Daytona](https://www.daytona.io/docs/en/) | Managed sandbox infrastructure with SDK/API, process and filesystem tools, snapshots, and credential mediation | Stateful cloud Agent workspaces | tinybox deliberately avoids becoming a cloud control plane; it targets one machine or one tenant VM. |
| [Modal Sandboxes](https://modal.com/docs/guide/sandboxes) | Cloud sandbox API with lifecycle, streaming exec, persistence, resources, and network controls | Programmatic untrusted-code execution | tinybox focuses on transparent Coding Agent tool replacement rather than a general hosted compute API. |

The MVP is trying to demonstrate a narrower user benefit:

- an Agent can obtain a persistent local container task without a project
  Dockerfile;
- repeated commands reuse an explicit environment while temporary process state
  is cleaned after each exec;
- Git manages source history while tinybox manages the local execution environment;
- OpenCode can adopt the task backend without changing its reasoning loop.

For detailed notes and the design lessons taken from these systems, see
[docs/COMPETITIVE_LANDSCAPE.md](docs/COMPETITIVE_LANDSCAPE.md).

## Current Status

**Phases 1–13 form the experimental runtime baseline; some features remain
partial. Persistent task/exec, host/rootfs/profile environments, and the generic
Agent CLI have passed M0–M2 root acceptance. OpenCode, Pi, and Codex adapters are
not yet implemented or supported.** See
[docs/PLAN.md](docs/PLAN.md) for the authoritative, line-referenced audit and
remediation roadmap. Per-phase status uses ✅ works / ⚠️ partial / ❌ broken.
The completed C0–C6 capability implementation record is in
[docs/CAPABILITY_PLAN.md](docs/CAPABILITY_PLAN.md). New product work follows
[docs/PRODUCT_PLAN.md](docs/PRODUCT_PLAN.md).

Capability-track status: C0–C6 complete. `--policy` enforces resource and
Landlock filesystem ceilings; allowlisted TCP egress traverses an in-sandbox
CONNECT helper and host broker while direct sockets remain unrouted. The daemon
exposes bounded per-sandbox audit events and summaries. Daemon policies can use
CAS-protected phase transitions to atomically replace broker rules and cgroup
limits. `scripts/tinybox-agent-tool` routes high-risk tools into tinybox, while
`tinybox agent-host` applies the same immutable FS ceiling to a host Agent.

Agent integration examples:

```bash
TINYBOX_POLICY=/orchestrator/policies/task.json \
  TINYBOX_BIN=./target/release/tinybox \
  ./scripts/tinybox-agent-tool bash -lc 'cargo test'

tinybox agent-host --policy /orchestrator/policies/task.json -- agent-command
sudo ./scripts/run_capability_workloads.sh
sudo ./scripts/benchmark_capability.sh 20
```

The required bare-versus-protected OpenCode integration and its acceptance
matrix are specified in [docs/OPENCODE_DEMO.md](docs/OPENCODE_DEMO.md). It is a
design specification, not a claim that the adapter already exists.

Keep the selected policy outside Agent-writable paths. The host launcher enforces
only the immutable filesystem ceiling; high-risk execution still belongs in the
sandbox wrapper.

### Feature Status (honest)

| Phase | Feature | Status | Notes |
|-------|---------|--------|-------|
| 1 | Project skeleton + CLI + subprocess execution | ✅ | — |
| 2 | Namespace isolation (PID/mount/UTS/Net) | ✅ | OCI subsets are typed and fail closed (C0) |
| 3 | Overlayfs rootfs + pivot_root | ✅ | special-FS setup is fail closed (C0) |
| 4 | cgroup resource limits (CPU/memory/pids) | ⚠️ | no v2 validation, `swap.max` hardcoded, no controller enabling (P2-2) |
| 5 | seccomp + capabilities hardening | ✅ | `clone` flag-masked; escape syscalls removed; bounding set cleared (M1) |
| 6 | OCI Bundle support (config.json subset) | ⚠️ | typed namespace subset; user namespace explicitly unsupported |
| 7 | Network namespace + policy broker | ✅ | no NIC; exact host/port CONNECT allowlist; direct sockets unrouted |
| 8 | HTTP API + daemon mode + Prometheus metrics | ⚠️ | bounded audit API added; persistence/auth remain open |
| 9 | Local image management (import/list/remove/run --image) | ⚠️ | no content addressing, no layering, no metadata (P2-3) |
| 10 | Docker Registry image pull | ⚠️ | in-memory blobs (OOM risk); never fetches config blob; Docker Hub only (P2-4) |
| 11 | ~~Network bridge + port mapping~~ | 🗑 | removed in M1 (Option A — contradicted design & leaked to host) |
| 12 | Volume mounting (bind mounts) | ✅ | pivot-before-bind ordering, symlink checks, real read-only remount |
| 13 | Exec into running containers | ⚠️ | direct `setns` and basic PID check; reuses a TTY but does not allocate a PTY |

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  tinybox CLI                      │
│  (run, daemon)                          │
└──────────┬───────────────────────────┬────────────┘
           │                           │
           ▼                           ▼
┌─────────────────────┐   ┌─────────────────────────┐
│  Direct Mode         │   │  Daemon Mode             │
│  (single sandbox)    │   │  (axum HTTP API)         │
│                      │   │  POST /api/sandboxes     │
│                      │   │  GET  /api/sandboxes     │
│                      │   │  DELETE /api/sandboxes   │
│                      │   │  GET  /metrics           │
└──────────┬───────────┘   └───────────┬─────────────┘
           │                           │
           └──────────┬────────────────┘
                      ▼
┌──────────────────────────────────────────────────┐
│              Sandbox Instance                     │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐  │
│  │ PID NS   │ │ Mount NS │ │ UTS NS           │  │
│  │ (isolate │ │ (private │ │ (hostname        │  │
│  │ process) │ │  rootfs) │ │  isolation)      │  │
│  └──────────┘ └──────────┘ └──────────────────┘  │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐  │
│  │ Net NS   │ │ cgroup   │ │ seccomp + caps   │  │
│  │ (proxy)  │ │ (mem/CPU)│ │ (syscall filter) │  │
│  └──────────┘ └──────────┘ └──────────────────┘  │
│  ┌──────────────────────────────────────────────┐ │
│  │ Overlayfs rootfs (COW, auto-cleanup)         │ │
│  └──────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- Rust toolchain (1.70+)
- Linux kernel 5.10+ (WSL2 works)
- Root privileges (for namespace operations)

### Build

```bash
cargo build --release
```

### Prepare a rootfs (one-time)

```bash
docker pull alpine:latest
docker export $(docker create alpine:latest) -o /tmp/alpine.tar
mkdir -p /tmp/alpine-rootfs && tar -xf /tmp/alpine.tar -C /tmp/alpine-rootfs
```

### Run a sandbox

```bash
# Basic execution
sudo ./target/release/tinybox run --root /tmp/alpine-rootfs -- /bin/sh

# With resource limits (Docker-style flags)
sudo ./target/release/tinybox run \
  --root /tmp/alpine-rootfs \
  --memory 256m \
  --cpus 0.5 \
  --pids-limit 100 \
  -- /bin/sh

# Bypass seccomp (dangerous, for debugging)
sudo ./target/release/tinybox run --dangerous -- /bin/sh
```

### CLI Commands

```text
tinybox run [OPTIONS] -- <COMMAND>...
tinybox daemon [--listen <ADDRESS>]
tinybox exec --pid <PID> -- <COMMAND>...
tinybox agent-host --policy <PATH> -- <AGENT>...
```
```text
USAGE:
    tinybox run [OPTIONS] -- <COMMAND>...

OPTIONS:
    --root <PATH>           Rootfs directory
    --hostname <NAME>       Set hostname in UTS namespace
    -m, --memory <LIMIT>    Memory limit (e.g., 256m, 1g)
    --cpus <NUM>            CPU limit (e.g., 0.5 = 50% of one core)
    --cpu-quota <MICROS>    CPU quota in microseconds
    --cpu-period <MICROS>   CPU period in microseconds (default: 100000)
    --pids-limit <NUM>      Maximum number of processes
    --dangerous             Disable seccomp and capability restrictions
    --policy <PATH>         Capability descriptor; network uses the policy broker
    --proxy <URL>           Legacy proxy environment injection only
    -v, --volume <VOLUME>   Bind mount volume (host:container[:ro])
    --oci <PATH>            OCI bundle path (Phase 6)
    --image <NAME>          Run from imported image (Phase 9)

EXAMPLES:
    tinybox run -- echo "hello"
    tinybox run --root /tmp/alpine-rootfs -- /bin/sh
    tinybox run --image alpine -- /bin/sh
    tinybox run --policy ./policy.json -- curl https://allowed.example
    tinybox run -v /host/path:/container/path -- ls /container/path
    tinybox run -v /data:/data:ro -- cat /data/file.txt
    tinybox run --memory 64m -- python3 -c "a = bytearray(200*1024*1024)"
    tinybox run --cpus 0.1 -- stress --cpu 1

EXEC INTO RUNNING CONTAINER:
    tinybox exec --pid <PID> -- /bin/sh
    tinybox exec --pid <PID> -- cat /proc/1/status

IMAGE MANAGEMENT:
    tinybox image import <TAR> --alias <NAME>   Import a rootfs tar as an image
    tinybox image pull <IMAGE>[:TAG]            Pull image from Docker Registry
    tinybox image list                          List imported images
    tinybox image remove <NAME>                 Remove an imported image
```

## Security Model

tinybox implements defense-in-depth with four layers of isolation:

### 1. Namespace Isolation (Phase 2)

- **PID namespace**: Process tree isolation
- **Mount namespace**: Private mount table with `MS_REC|MS_PRIVATE`
- **UTS namespace**: Hostname isolation

### 2. Resource Limits (Phase 4)

- **Memory**: cgroup v2 `memory.max` + `memory.swap.max=0`
- **CPU**: cgroup v2 `cpu.max` (quota/period)
- **PIDs**: cgroup v2 `pids.max`

### 3. Capability Dropping (Phase 5)

Drops 14 selected dangerous capabilities before exec and from the bounding set;
see `DANGEROUS_CAPS` in `src/seccomp.rs` for the authoritative list.

### 4. Seccomp Filtering (Phase 5)

- Whitelist of ~200 safe syscalls
- Blocked syscalls trigger `SIGSYS` (exit code 159)
- `--dangerous` flag disables filtering

## Testing

```bash
# Run all tests
cargo test

# Run specific phase tests
cargo test --test phase1
cargo test --test phase2
cargo test --test phase3
cargo test --test phase4
cargo test --test phase5
cargo test --test phase6
cargo test --test phase7
cargo test --test phase8

# Run acceptance tests (requires root)
sudo ./scripts/test_phase1.sh
sudo ./scripts/test_phase2.sh
sudo ./scripts/test_phase3.sh
sudo ./scripts/test_phase4.sh
sudo ./scripts/test_phase5.sh
sudo ./scripts/test_phase6.sh
sudo ./scripts/test_phase7.sh
sudo ./scripts/test_phase8.sh
sudo ./scripts/test_phase9.sh
sudo ./scripts/test_phase10.sh
```

When run as a non-root user, privileged integration tests return early and are
reported as passed. Use the root-only acceptance scripts for runtime claims,
and report those results separately from unit tests and lint.

## Development Phases

| Phase | Feature | Lines (Rust) | Status |
|-------|---------|-------------|--------|
| 1 | Project skeleton + CLI + subprocess execution | ~150 | ✅ |
| 2 | Namespace isolation (PID/mount/UTS/Net) | ~200 | ✅ |
| 3 | Overlayfs rootfs + pivot_root | ~150 | ✅ |
| 4 | cgroup resource limits (CPU/memory/pids) | ~200 | ⚠️ |
| 5 | seccomp + capabilities hardening | ~250 | ✅ |
| 6 | OCI Bundle support (config.json parsing) | ~350 | ⚠️ |
| 7 | Network namespace + policy broker | ~250 | ✅ |
| 8 | HTTP API + daemon mode + Prometheus metrics | ~350 | ⚠️ |
| 9 | Local image management (import/list/remove) | ~150 | ⚠️ |
| 10 | Docker Registry image pull | ~150 | ⚠️ |
| 11 | ~~Network bridge + port mapping~~ | ~200 | 🗑 removed |
| 12 | Volume mounting (bind mounts) | ~50 | ✅ |
| 13 | Exec into running containers | ~50 | ⚠️ |

Status: ✅ works · ⚠️ partial (open P1/P2) · ❌ broken (open P0 or fails
acceptance) · 🗑 removed. Phase 11 was removed in M1 (Option A) — it
contradicted the documented "no bridge" design and leaked to the host netns.
See [docs/PLAN.md](docs/PLAN.md) for the per-phase open-item mapping.

## Known Issues

> This section summarizes the open defects; see [docs/PLAN.md](docs/PLAN.md) for the
> full, line-referenced list with fixes and the remediation roadmap.

### ✅ P0 — Isolation / Security (FIXED in M1, 2026-08-16)
All four P0 escape/leak holes are closed:
- ~~`--network bridge` configures the host instead of the sandbox~~ → the
  bridge/veth/NAT path was **removed** (`src/network.rs` deleted, Option A).
- ~~`--proxy` shared the host netns~~ → fixed: the default path unshares
  `CLONE_NEWNET`; C3 policy egress now uses a CONNECT helper and host broker.
- ~~seccomp allow-list has escape primitives~~ → `clone` is now
  flag-masked (forbids `CLONE_NEW*`); `open_by_handle_at`/`process_vm_*`/
  `perf_event_open`/NUMA-IO setters removed; `CAP_DAC_READ_SEARCH`/
  `CAP_NET_RAW`/`CAP_SETFCAP`/etc. now dropped.
- ~~capability bounding set never cleared~~ → `drop_capabilities` now calls
  `prctl(PR_CAPBSET_DROP)` for each dangerous cap.

### Correctness gaps reopened by the M2 re-audit
- OCI namespace subsets are typed and fail closed; user namespace remains
  explicitly unsupported rather than silently ignored.
- ~~`ip`/`iptables` silent failure~~ → resolved (network.rs removed in M1).
- daemon separates pre-fork `failed`, child `setup_failed`, and payload exit status.
- ~~daemon `CreateRequest` can't set options~~ → extended with cpus/pids/volumes/hostname/env/root_readonly; `dangerous:true` rejected remotely (P1-4).
- `exec` uses direct `setns` and a cgroup-name check, but has no PTY allocator
  and its privileged acceptance coverage remains limited.

### P2 — Shallow Features
- `/dev/mqueue` remains absent; policy-critical special mounts otherwise fail closed.
- `--proxy` remains legacy env injection; policy networking uses the C3 broker.
- cgroup validates v2 and requested controllers; controller enabling and the
  hardcoded `swap.max=0` policy remain P2 work.
- images: no content addressing, no layering, no metadata (P2-3).
- registry pull: entire layers loaded into RAM; config blob never fetched; no digest verification; Docker Hub only (P2-4).
- daemon: no persistence, no auth, no log/exec endpoints (P2-5).

### WSL2 Limitations

- **Mount propagation**: Requires `mount -t devpts devpts /dev/pts` if `/dev/pts` is missing
- **Cgroup swap**: WSL2 has swap enabled by default; tinybox sets `memory.swap.max=0` to ensure OOM kill works
- **Performance**: Some namespace operations may be slower than native Linux

### Root Required

Most operations require root privileges. Future phases will explore user namespaces for rootless operation.

## Design Decisions

See [AGENTS.md](AGENTS.md) for development conventions, constraints, and decision log.

## License

MIT

## Related Projects

- [mini-infer](https://github.com/Deyu-Kong/mini-infer): C++/CUDA LLM inference engine from scratch
- [runc](https://github.com/opencontainers/runc): The reference OCI runtime
- [Firecracker](https://github.com/firecracker-microvm/firecracker): Secure containerization with microVMs
