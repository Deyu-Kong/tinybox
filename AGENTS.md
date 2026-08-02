# Agent Guidelines for tinybox

## Project Overview

tinybox is a from-scratch Rust implementation of a Linux sandbox runtime, similar to `runc` but simplified and focused on Agent workloads. It is built incrementally across 8 phases, each producing a runnable, verifiable deliverable.

## Conventions

### Language & Style
- **Rust edition**: 2021
- **Formatting**: `rustfmt` with default settings
- **Linting**: `clippy` with no warnings allowed
- **Naming**: snake_case for functions/variables, CamelCase for types, SCREAMING_SNAKE_CASE for constants
- **Error handling**: Use `anyhow::Result` for CLI/API code, custom error types for library code
- **Unsafe**: Minimize `unsafe`. Only use for FFI or direct syscall wrappers. Document safety invariants in `// SAFETY:` comments

### Dependencies
Prefer minimal dependencies. Approved crates:
- `clap` (CLI parsing)
- `serde` / `serde_json` / `serde_yaml` (config serialization)
- `axum` (HTTP API)
- `nix` (Linux syscall wrappers, for namespaces, pivot_root, etc.)
- `anyhow` / `thiserror` (error handling)
- `tokio` (async runtime for daemon mode)
- `tracing` / `tracing-subscriber` (logging)
- `libc` (raw FFI only when nix doesn't cover it)

Do NOT add dependencies without a clear reason. Avoid full-blown container runtimes, virtualization libraries, or OCI SDKs.

### Linux-Specific Code
- `cfg!(target_os = "linux")` guards for all Linux-specific code
- Panic with a clear message on non-Linux platforms
- cgroup v2 only (no v1 support)
- Assume kernel 5.10+ (Ubuntu 20.04 LTS baseline)

### Testing
- **Unit tests**: `#[cfg(test)] mod tests { ... }` alongside each module
- **Integration tests**: `tests/` directory, one file per phase
- **Acceptance tests**: Shell scripts in `scripts/` that verify each phase's acceptance criteria
- Run tests with: `cargo test && cargo clippy -- -D warnings`
- Run acceptance tests with: `SUDO_ASKPASS=.sudo-askpass.sh sudo -A ./scripts/test_phaseN.sh`
- **Sudo setup**: The `.sudo-askpass.sh` script provides sudo password for automated testing (password: kdy)

### Git Workflow
- One commit per completed phase
- Commit message format: `phase N: <short description>`
- Tag each phase completion: `git tag v0.N`

## Phase Dependencies

Each phase depends on the previous one. Do NOT skip ahead. The phases are:

```
1 (skeleton) → 2 (namespaces) → 3 (overlayfs) → 4 (cgroups)
                                                        ↓
                                          5 (seccomp) → 6 (OCI) → 7 (network) → 8 (API)
```

- Phase 5 and 6 can be swapped (seccomp before or after OCI)
- Phase 7 (network) can be done in parallel with phase 6 (OCI) if preferred
- Phase 8 is the finale and depends on all previous phases

## Verification

Each phase has explicit acceptance criteria. Verify them manually before marking a phase complete:

### Phase 1
```bash
tinybox run -- echo hello          # → "hello"
tinybox run -- sh -c "exit 42"    # → exit code 42
```

### Phase 2
```bash
tinybox run -- ps aux              # → only 2-3 processes
tinybox run -- id                  # → uid=0(root)
tinybox run --hostname sbox1 -- hostname  # → "sbox1"
```

### Phase 3
```bash
# Prepare rootfs first
tinybox run --root /tmp/alpine-rootfs -- ls /  # → alpine fs
tinybox run --root /tmp/alpine-rootfs -- sh -c "echo hi > /t && cat /t"  # → "hi", host file doesn't exist
```

### Phase 4
```bash
tinybox run --mem-limit 64M -- sh -c "dd if=/dev/zero of=/dev/null bs=1M count=200"  # → OOM killed
```

### Phase 5
```bash
tinybox run -- reboot              # → SIGSYS
tinybox run -- mount -t tmpfs none /tmp  # → fails
tinybox run --dangerous -- mount -t tmpfs none /tmp  # → succeeds
```

### Phase 6
```bash
# Build OCI bundle
mkdir -p /tmp/oci-bundle/rootfs
cp -a /tmp/alpine-rootfs/. /tmp/oci-bundle/rootfs/
cat > /tmp/oci-bundle/config.json <<'EOF'
{"process":{"args":["/bin/sh","-c","echo hello-oci"],"env":["PATH=/usr/bin"]},"root":{"path":"rootfs","readonly":true},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"}]}}
EOF
tinybox run --oci /tmp/oci-bundle   # → "hello-oci"
```

### Phase 7
```bash
tinybox run -- ping 8.8.8.8         # → network unreachable
tinybox run --proxy http://127.0.0.1:8080 -- wget -q -O- http://example.com  # → succeeds
```

### Phase 8
```bash
tinybox daemon --listen 127.0.0.1:8080 &
curl -X POST http://127.0.0.1:8080/api/sandboxes -H "Content-Type: application/json" -d '{"rootfs":"/tmp/alpine-rootfs","mem_limit_mb":256,"command":["sleep","30"]}'  # → {"id":"sb-..."}
curl http://127.0.0.1:8080/metrics  # → Prometheus metrics
```

## Constraints

### What NOT to do
- Do NOT implement a full OCI runtime (no need to handle all 50+ config.json fields; implement only the core 10)
- Do NOT add GPU support (no passthrough, no CUDA)
- Do NOT implement image pulling (rely on `docker export` or pre-existing rootfs)
- Do NOT implement multi-node orchestration (single-host only)
- Do NOT handle SELinux or AppArmor (seccomp is sufficient)
- Do NOT implement user-mode networking (no TUN/TAP, no bridge)
- Do NOT use Docker, containerd, or runc libraries (this is a from-scratch project)
- Do NOT add Windows/macOS support (Linux-only)

### What to prioritize
- **Correctness**: The sandbox must actually isolate. Leaking processes to the host is a bug.
- **Safety**: Default seccomp policy must prevent escape. `--dangerous` is opt-in.
- **Measurability**: Every optimization must be backed by a benchmark number.
- **Simplicity**: ~2000 lines total Rust. Favor readable code over clever abstractions.

## Decision Log

### 2026-08-02: Project inception
- **Language**: Rust (JD requirement: "倾向 Rust / C / Python")
- **Isolation model**: Process-level (namespaces + cgroups + seccomp), NOT VMM-based
- **Rootfs**: Overlayfs with COW, auto-cleanup on exit
- **Network**: Proxy-based isolation (no real NIC in sandbox, all traffic through host proxy)
- **OCI compatibility**: Phase 6, support core config.json fields only, not full spec
- **CLI name**: `tinybox` (previously considered: sandbox-rs, jail, sbox, cell)

### 2026-08-02: Phase ordering
- OCI support moved to Phase 6 (after core isolation works, before network)
- Rationale: OCI config.json wraps all isolation features, so it should be added after they work individually
- Network is Phase 7 because it's the most complex and can be developed independently

## Related Projects

- [mini-infer](https://github.com/Deyu-Kong/mini-infer): C++/CUDA LLM inference engine from scratch (same "from scratch" philosophy)
- [runc](https://github.com/opencontainers/runc): The reference OCI runtime (tinybox is a simplified educational reimplementation)