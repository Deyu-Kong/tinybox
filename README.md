# tinybox — Agent Sandbox Runtime

A minimal, secure sandbox runtime for running AI Agents in isolated environments, built from scratch in Rust.

![Phase Progress](https://img.shields.io/badge/phases-1--13-audited-yellow)
![Lines of Code](https://img.shields.io/badge/Rust_LOC-2306-orange)
![License](https://img.shields.io/badge/license-MIT-green)
![Status](https://img.shields.io/badge/status-experimental%2C%20open%20correctness%20gaps-orange)

> ⚠️ **Experimental, rootful runtime; not a production security boundary.**
> The original M1 P0 findings were fixed, but an M2 re-audit found open
> correctness gaps in OCI namespace subsets, daemon failure reporting,
> special-filesystem setup, proxy connectivity, and read-only volumes. See
> [docs/PLAN.md](docs/PLAN.md) for the authoritative status.

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

**Phases 1–13 have code, but several are partial after the M2 re-audit.** See
[docs/PLAN.md](docs/PLAN.md) for the authoritative, line-referenced audit and
remediation roadmap. Per-phase status uses ✅ works / ⚠️ partial / ❌ broken.
The implementation sequence for Agent-oriented capability management is in
[docs/CAPABILITY_PLAN.md](docs/CAPABILITY_PLAN.md).

### Feature Status (honest)

| Phase | Feature | Status | Notes |
|-------|---------|--------|-------|
| 1 | Project skeleton + CLI + subprocess execution | ✅ | — |
| 2 | Namespace isolation (PID/mount/UTS/Net) | ⚠️ | default path isolates; OCI subsets need validation |
| 3 | Overlayfs rootfs + pivot_root | ⚠️ | special-FS setup ignores several mount failures |
| 4 | cgroup resource limits (CPU/memory/pids) | ⚠️ | no v2 validation, `swap.max` hardcoded, no controller enabling (P2-2) |
| 5 | seccomp + capabilities hardening | ✅ | `clone` flag-masked; escape syscalls removed; bounding set cleared (M1) |
| 6 | OCI Bundle support (config.json subset) | ⚠️ | fields parse, but namespace/user semantics are incomplete |
| 7 | Network namespace + proxy environment | ⚠️ | isolated netns and env only; no host-proxy transport |
| 8 | HTTP API + daemon mode + Prometheus metrics | ⚠️ | pre-fork failures tracked; child setup failures can appear completed |
| 9 | Local image management (import/list/remove/run --image) | ⚠️ | no content addressing, no layering, no metadata (P2-3) |
| 10 | Docker Registry image pull | ⚠️ | in-memory blobs (OOM risk); never fetches config blob; Docker Hub only (P2-4) |
| 11 | ~~Network bridge + port mapping~~ | 🗑 | removed in M1 (Option A — contradicted design & leaked to host) |
| 12 | Volume mounting (bind mounts) | ⚠️ | read-only remount is incomplete |
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
    tinybox run --proxy http://proxy.example:8080 -- env
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
| 2 | Namespace isolation (PID/mount/UTS/Net) | ~200 | ⚠️ |
| 3 | Overlayfs rootfs + pivot_root | ~150 | ⚠️ |
| 4 | cgroup resource limits (CPU/memory/pids) | ~200 | ⚠️ |
| 5 | seccomp + capabilities hardening | ~250 | ✅ |
| 6 | OCI Bundle support (config.json parsing) | ~350 | ⚠️ |
| 7 | Network namespace + proxy environment | ~250 | ⚠️ |
| 8 | HTTP API + daemon mode + Prometheus metrics | ~350 | ⚠️ |
| 9 | Local image management (import/list/remove) | ~150 | ⚠️ |
| 10 | Docker Registry image pull | ~150 | ⚠️ |
| 11 | ~~Network bridge + port mapping~~ | ~200 | 🗑 removed |
| 12 | Volume mounting (bind mounts) | ~50 | ⚠️ |
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
  `CLONE_NEWNET`. Proxy connectivity itself remains unimplemented; see below.
- ~~seccomp allow-list has escape primitives~~ → `clone` is now
  flag-masked (forbids `CLONE_NEW*`); `open_by_handle_at`/`process_vm_*`/
  `perf_event_open`/NUMA-IO setters removed; `CAP_DAC_READ_SEARCH`/
  `CAP_NET_RAW`/`CAP_SETFCAP`/etc. now dropped.
- ~~capability bounding set never cleared~~ → `drop_capabilities` now calls
  `prctl(PR_CAPBSET_DROP)` for each dangerous cap.

### Correctness gaps reopened by the M2 re-audit
- OCI parses `linux.namespaces`/`root.readonly`/`process.cwd`/`process.user`,
  but mount initialization is not safe for every namespace subset and the
  requested user namespace is ignored.
- ~~`ip`/`iptables` silent failure~~ → resolved (network.rs removed in M1).
- daemon reports pre-fork errors as `failed`, but child setup/exec failures can
  still be reported as `completed` with exit code 1.
- ~~daemon `CreateRequest` can't set options~~ → extended with cpus/pids/volumes/hostname/env/root_readonly; `dangerous:true` rejected remotely (P1-4).
- `exec` uses direct `setns` and a cgroup-name check, but has no PTY allocator
  and its privileged acceptance coverage remains limited.

### P2 — Shallow Features
- rootfs has `/dev`, `/tmp`, empty `/sys`, and `/proc` setup, but several mount
  failures are ignored and `/dev/mqueue` is absent.
- `--proxy` only injects environment variables into an isolated netns; no
  transport connects that netns to a host proxy.
- read-only bind volumes need a proper bind remount.
- cgroup: no v2 validation, no controller enabling, `swap.max` hardcoded (P2-2).
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
