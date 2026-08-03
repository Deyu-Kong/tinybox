# tinybox — Agent Sandbox Runtime

A minimal, secure sandbox runtime for running AI Agents in isolated environments, built from scratch in Rust.

![Phase Progress](https://img.shields.io/badge/phase-7%2F8-blue)
![Lines of Code](https://img.shields.io/badge/LOC-973-orange)
![License](https://img.shields.io/badge/license-MIT-green)

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

**Phase 7 of 8 completed** (973 lines of Rust code)

### Completed Features

- ✅ **Phase 1**: Project skeleton + CLI + subprocess execution
- ✅ **Phase 2**: Namespace isolation (PID/mount/UTS)
- ✅ **Phase 3**: Overlayfs rootfs + pivot_root
- ✅ **Phase 4**: cgroup resource limits (CPU/memory/pids)
- ✅ **Phase 5**: seccomp + capabilities hardening
- ✅ **Phase 6**: OCI Bundle support (config.json parsing)
- ✅ **Phase 7**: Network namespace + proxy environment

### In Progress

- ⏳ **Phase 8**: HTTP API + daemon mode + Prometheus metrics

## Architecture

```
┌──────────────────────────────────────────────────┐
│                  tinybox CLI                      │
│  (clap: run, daemon, list, kill, stats)           │
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

### CLI Reference

```
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
    --proxy <URL>           HTTP proxy for network isolation
    --oci <PATH>            OCI bundle path (Phase 6)

EXAMPLES:
    tinybox run -- echo "hello"
    tinybox run --root /tmp/alpine-rootfs -- /bin/sh
    tinybox run --memory 64m -- python3 -c "a = bytearray(200*1024*1024)"
    tinybox run --cpus 0.1 -- stress --cpu 1
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

# Run acceptance tests (requires root)
sudo ./scripts/test_phase1.sh
sudo ./scripts/test_phase2.sh
sudo ./scripts/test_phase3.sh
sudo ./scripts/test_phase4.sh
sudo ./scripts/test_phase5.sh
sudo ./scripts/test_phase6.sh
sudo ./scripts/test_phase7.sh
```

## Development Phases

| Phase | Feature | Lines (Rust) | Status |
|-------|---------|-------------|--------|
| 1 | Project skeleton + CLI + subprocess execution | ~150 | ✅ |
| 2 | Namespace isolation (PID/mount/UTS) | ~200 | ✅ |
| 3 | Overlayfs rootfs + pivot_root | ~150 | ✅ |
| 4 | cgroup resource limits (CPU/memory/pids) | ~200 | ✅ |
| 5 | seccomp + capabilities hardening | ~250 | ✅ |
| 6 | OCI Bundle support (config.json parsing) | ~350 | ✅ |
| 7 | Network namespace + proxy environment | ~250 | ✅ |
| 8 | HTTP API + daemon mode + Prometheus metrics | ~350 | ⏳ |

## Known Issues

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