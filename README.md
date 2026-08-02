# tinybox — Agent Sandbox Runtime

A minimal, secure sandbox runtime for running AI Agents in isolated environments, built from scratch in Rust.

## Motivation

This project is a companion to the DeepSeek Agent Infra position. It demonstrates understanding of Linux container isolation primitives by building a lightweight sandbox runtime from scratch, similar in spirit to how [mini-infer](https://github.com/Deyu-Kong/mini-infer) builds an LLM inference engine from scratch.

The project covers all six directions of the Agent Infra JD:
- **Containerization**: Linux namespaces, cgroups, seccomp, capabilities
- **Virtualization**: Process-level isolation without VMM overhead
- **Ephemeral storage**: Overlayfs rootfs with auto-cleanup
- **Virtual networking**: Proxy-based network isolation (no real NIC in sandbox)
- **Cloud services**: HTTP management API, daemon mode, Prometheus metrics
- **Observability**: /metrics endpoint, benchmark comparisons

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
│  │ PID NS   │ │ Mount NS │ │ User NS          │  │
│  │ (isolate │ │ (private │ │ (root in sandbox │  │
│  │ process) │ │  rootfs) │ │  ≠ root on host) │  │
│  └──────────┘ └──────────┘ └──────────────────┘  │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐  │
│  │ Net NS   │ │ cgroup   │ │ seccomp          │  │
│  │ (lo only)│ │ (mem/CPU)│ │ (syscall filter) │  │
│  └──────────┘ └──────────┘ └──────────────────┘  │
│  ┌──────────────────────────────────────────────┐ │
│  │ Overlayfs rootfs (COW, auto-cleanup)         │ │
│  └──────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

## Development Phases

| Phase | Feature | Lines (Rust) | Status |
|-------|---------|-------------|--------|
| 1 | Project skeleton + CLI + subprocess execution | ~150 | |
| 2 | Namespace isolation (PID/mount/user/UTS/IPC) | ~200 | |
| 3 | Overlayfs rootfs + pivot_root | ~150 | |
| 4 | cgroup resource limits (CPU/memory) | ~150 | |
| 5 | seccomp + capabilities hardening | ~200 | |
| 6 | OCI Bundle support (config.json parsing) | ~350 | |
| 7 | Network isolation + proxy networking | ~250 | |
| 8 | HTTP API + daemon mode + Prometheus metrics + benchmark | ~350 | |

## Quick Start

```bash
# Prerequisites: Rust toolchain, Linux (WSL2 works)
cargo build --release

# Prepare a rootfs (one-time)
docker pull alpine:latest
docker export $(docker create alpine:latest) -o /tmp/alpine.tar
mkdir -p /tmp/alpine-rootfs && tar -xf /tmp/alpine.tar -C /tmp/alpine-rootfs

# Run a sandbox
./target/release/tinybox run --root /tmp/alpine-rootfs -- /bin/sh

# With resource limits
./target/release/tinybox run --root /tmp/alpine-rootfs --mem-limit 256M --cpu-quota 50000 -- /bin/sh

# Daemon mode with HTTP API
./target/release/tinybox daemon --listen 127.0.0.1:8080
```

## Design Decisions

See [AGENTS.md](AGENTS.md) for development conventions, constraints, and decision log.