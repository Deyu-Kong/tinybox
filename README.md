# tinybox — Agent Sandbox Runtime

A minimal, secure sandbox runtime for running AI Agents in isolated environments, built from scratch in Rust.

![Phase Progress](https://img.shields.io/badge/phase-13%2F8-yellow)
![Lines of Code](https://img.shields.io/badge/LOC-2004-orange)
![License](https://img.shields.io/badge/license-MIT-green)
![Status](https://img.shields.io/badge/status-P0%20fixed%2C%20hardening%20incomplete-yellow)

> ⚠️ **P0 isolation holes are fixed (M1, 2026-08-16), but hardening is
> incomplete.** tinybox now properly isolates via namespaces + cgroups +
> seccomp + caps (the `tinybox run` path is a defensible barrier), but it is
> still rootful, lacks `/dev`/`/tmp`/`/sys` hardening, and OCI field-honoring
> is incomplete. See [docs/PLAN.md](docs/PLAN.md) for the full audit and the
> remaining P1/P2 items.

## Motivation

This project is a companion to the DeepSeek Agent Infra position. It demonstrates understanding of Linux container isolation primitives by building a lightweight sandbox runtime from scratch, similar in spirit to how [mini-infer](https://github.com/Deyu-Kong/mini-infer) builds an LLM inference engine from scratch.

The project covers all six directions of the Agent Infra JD:
- **Containerization**: Linux namespaces, cgroups, seccomp, capabilities
- **Virtualization**: Process-level isolation without VMM overhead
- **Ephemeral storage**: Overlayfs rootfs with auto-cleanup
- **Virtual networking**: Proxy-based network isolation (no real NIC in sandbox)
- **Cloud services**: HTTP management API, daemon mode, Prometheus metrics
- **Observability**: /metrics endpoint, benchmark comparisons

## Current Status

**13 phases implemented (2004 lines of Rust). P0 isolation holes fixed
(M1, 2026-08-16); remaining issues are P1/P2 (correctness/depth), not escape
holes.** See [docs/PLAN.md](docs/PLAN.md) for the authoritative, line-referenced
audit and remediation roadmap. Per-phase status uses ✅ works / ⚠️ partial /
❌ broken.

### Feature Status (honest)

| Phase | Feature | Status | Notes |
|-------|---------|--------|-------|
| 1 | Project skeleton + CLI + subprocess execution | ✅ | — |
| 2 | Namespace isolation (PID/mount/UTS/Net) | ✅ | NEWNET now always unshared (M1) |
| 3 | Overlayfs rootfs + pivot_root | ⚠️ | no `/dev`, `/tmp`, `/sys`; OCI `readonly` ignored (P2-1) |
| 4 | cgroup resource limits (CPU/memory/pids) | ⚠️ | no v2 validation, `swap.max` hardcoded, no controller enabling (P2-2) |
| 5 | seccomp + capabilities hardening | ✅ | `clone` flag-masked; escape syscalls removed; bounding set cleared (M1) |
| 6 | OCI Bundle support (config.json parsing) | ❌ | only 3 of ~10 claimed fields honored; `linux.namespaces` silently dropped (P1-1) |
| 7 | Network namespace + proxy environment | ✅ | `--proxy` now real isolation (loopback-only + env); bridge removed (M1) |
| 8 | HTTP API + daemon mode + Prometheus metrics | ⚠️ | limited `CreateRequest`; failed sandboxes miscounted as completed (P1-3, P1-4, P2-5) |
| 9 | Local image management (import/list/remove/run --image) | ⚠️ | no content addressing, no layering, no metadata (P2-3) |
| 10 | Docker Registry image pull | ⚠️ | in-memory blobs (OOM risk); never fetches config blob; Docker Hub only (P2-4) |
| 11 | ~~Network bridge + port mapping~~ | 🗑 | removed in M1 (Option A — contradicted design & leaked to host) |
| 12 | Volume mounting (bind mounts) | ✅ | — |
| 13 | Exec into running containers | ⚠️ | 23-line `nsenter` wrapper; missing `-i/-U/-C`, no TTY, no PID validation (P1-5) |

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
    --proxy <URL>           HTTP proxy (sandbox netns is loopback-only; proxy
                            env vars are set for cooperating clients)
    -v, --volume <VOLUME>   Bind mount volume (host:container[:ro])
    --oci <PATH>            OCI bundle path (Phase 6)
    --image <NAME>          Run from imported image (Phase 9)

EXAMPLES:
    tinybox run -- echo "hello"
    tinybox run --root /tmp/alpine-rootfs -- /bin/sh
    tinybox run --image alpine -- /bin/sh
    tinybox run --proxy http://127.0.0.1:8080 -- wget -q -O- http://example.com
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

Drops dangerous capabilities before exec:
- `CAP_SYS_ADMIN`, `CAP_NET_ADMIN`, `CAP_SYS_MODULE`
- `CAP_SYS_RAWIO`, `CAP_SYS_PTRACE`, `CAP_SYS_BOOT`
- `CAP_SYS_TIME`, `CAP_MKNOD`

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

## Development Phases

| Phase | Feature | Lines (Rust) | Status |
|-------|---------|-------------|--------|
| 1 | Project skeleton + CLI + subprocess execution | ~150 | ✅ |
| 2 | Namespace isolation (PID/mount/UTS/Net) | ~200 | ✅ |
| 3 | Overlayfs rootfs + pivot_root | ~150 | ⚠️ |
| 4 | cgroup resource limits (CPU/memory/pids) | ~200 | ⚠️ |
| 5 | seccomp + capabilities hardening | ~250 | ✅ |
| 6 | OCI Bundle support (config.json parsing) | ~350 | ❌ |
| 7 | Network namespace + proxy environment | ~250 | ✅ |
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
- ~~`--proxy` is not isolation~~ → the sandbox now **always** unshares
  `CLONE_NEWNET`; `--proxy` = loopback-only netns + env vars.
- ~~seccomp allow-list has escape primitives~~ → `clone` is now
  flag-masked (forbids `CLONE_NEW*`); `open_by_handle_at`/`process_vm_*`/
  `perf_event_open`/NUMA-IO setters removed; `CAP_DAC_READ_SEARCH`/
  `CAP_NET_RAW`/`CAP_SETFCAP`/etc. now dropped.
- ~~capability bounding set never cleared~~ → `drop_capabilities` now calls
  `prctl(PR_CAPBSET_DROP)` for each dangerous cap.

### P1 — Correctness / Contradictions
- **OCI support is shallow**: `linux.namespaces`, `root.readonly`, `mounts`,
  `process.{cwd,user}` are all silently ignored — only `args`/`env`/`root.path`
  are honored (3 of the claimed "core 10" fields).
- ~~`ip`/`iptables` non-zero exit silently treated as success~~ → resolved:
  `network.rs` was removed in M1.
- **daemon conflates failed and completed sandboxes**: `exit_code = result.ok()`
  marks crashes as `completed`; `/metrics` overcounts.
- **daemon `CreateRequest` cannot set cpus/volumes/...** — the API can't
  exercise most CLI features.
- **`exec` is a 23-line `nsenter` wrapper**: missing `-i/-U/-C`, no TTY, no
  validation that the PID is a tinybox sandbox.

### P2 — Shallow Features
- rootfs: no `/dev`, `/tmp` tmpfs, `/sys`; only `/proc` mounted.
- cgroup: no v2 validation, no controller enabling, `swap.max` hardcoded to 0.
- images: no content addressing, no layering, no metadata.
- registry pull: entire layers loaded into RAM; config blob never fetched
  (so pulled images have no default `Cmd`); no digest verification; Docker Hub only.
- daemon: no persistence, no auth, no log/exec endpoints.

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