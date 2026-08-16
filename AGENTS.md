# Agent Guidelines for tinybox

> **Read [PLAN.md](docs/PLAN.md) first.** It is the authoritative, line-referenced
> audit of the current codebase (P0–P3 issues + remediation roadmap). The
> per-phase status in docs/PLAN.md supersedes any "✅" in older docs: several
> phases have open P0/P1 items and must not be considered complete until the
> corresponding milestone in docs/PLAN.md is tagged.

## Project Overview

tinybox is a from-scratch Rust implementation of a Linux sandbox runtime, similar to `runc` but simplified and focused on Agent workloads. It is built incrementally across 8 phases, each producing a runnable, verifiable deliverable.

> **⚠️ Security status (2026-08-16): NOT a security boundary.** Four P0
> isolation holes are open — most importantly the `--network bridge` path
> mutates the host netns, and the seccomp allow-list contains escape
> primitives. Do not confine untrusted workloads until docs/PLAN.md Milestone M1
> is complete.

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
- `seccompiler` (seccomp BPF filter generation — used by Phase 5)
- `tar` / `flate2` (image tarball extraction — used by Phase 9/10)
- `reqwest` (Docker registry HTTP — used by Phase 10; blocking feature)

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
- **WSL2 fix**: If `sudo` fails with "unable to allocate pty", run: `sudo mount -t devpts devpts /dev/pts`

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
tinybox run --memory 64m -- sh -c "dd if=/dev/zero of=/dev/null bs=1M count=200"  # → OOM killed
```
> Note: the flag is `--memory`/`-m` (not `--mem-limit`), and the suffix is
> case-sensitive lowercase (`64m`, not `64M`).

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
> ⚠️ **P1-1 (open):** the `linux.namespaces`, `root.readonly`, `process.cwd`,
> and `process.user` fields are currently **silently ignored**. tinybox always
> creates the full namespace set regardless of the bundle config. The
> acceptance test passes only because it doesn't assert namespace *subsets*.

### Phase 7
```bash
tinybox run -- ping 8.8.8.8         # → network unreachable
tinybox run --proxy http://127.0.0.1:8080 -- wget -q -O- http://example.com  # → succeeds
```
> ⚠️ **P0-1 / P0-2 (open):** `--proxy` currently sets env vars only and does
> **not** unshare `CLONE_NEWNET`, so the sandbox shares the host netns — the
> `--proxy wget` acceptance passes because the host has network, not because
> traffic is proxied. The `ping` acceptance only holds when **neither**
> `--proxy` nor `--network` is passed. The `--network bridge` path
> additionally mutates the host netns (see P0-1 in docs/PLAN.md).

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

> **Constraint violation in the tree (2026-08-16):** Phase 11 (`src/network.rs`)
> implemented a bridge + veth + NAT networking path that **directly violates**
> the "no TUN/TAP, no bridge" rule above, and does so unsafely (see P0-1 in
> docs/PLAN.md). The decision to keep or remove it is recorded in the Decision Log
> below; until M1 lands, treat `--network bridge` as broken and dangerous.

### What to prioritize
- **Correctness**: The sandbox must actually isolate. Leaking processes to the host is a bug.
- **Safety**: Default seccomp policy must prevent escape. `--dangerous` is opt-in.
- **Measurability**: Every optimization must be backed by a benchmark number.
- **Simplicity**: ~2000 lines total Rust. Favor readable code over clever abstractions.

> **Safety status (2026-08-16):** the "default seccomp policy must prevent
> escape" priority is **currently not met** — P0-3 (escape primitives in the
> allow-list) and P0-4 (bounding set never cleared) are open.

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

### 2026-08-16: Codebase review and remediation plan
- **Outcome**: A line-level audit of all 11 source files (~2004 LOC) produced
  [PLAN.md](docs/PLAN.md) with 4 P0, 5 P1, 5 P2, and 6 P3 items.
- **Network design contradiction**: Phase 11 (`src/network.rs`) implemented a
  bridge + veth + NAT path that contradicts the 2026-08-02 "proxy-based, no
  bridge" decision and the "no TUN/TAP, no bridge" constraint. It also leaks
  to the host netns (P0-1). **Pending decision** (Milestone M1):
  - **Option A (recommended)**: remove `network.rs` entirely; restore the
    documented proxy-only design; `--proxy` gets a real `CLONE_NEWNET`
    (loopback-only) + env vars.
  - **Option B**: keep the bridge, fix the ordering bug, and update the
    design docs to permit bridge networking as an opt-in feature.
- **OCI support depth**: Phase 6 honors only `process.args`/`env` and
  `root.path` — `linux.namespaces` etc. are silently dropped (P1-1). The
  "core 10 fields" claim in this file is aspirational, not factual.
- **Seccomp escape primitives**: `clone` (unrestricted), `open_by_handle_at`,
  `process_vm_readv/writev`, `perf_event_open` are in the allow-list; the
  bounding set is never cleared (P0-3, P0-4).
- **Docs policy**: README and AGENTS.md now reflect real per-phase status
  (✅/⚠️/❌) and point to docs/PLAN.md as the source of truth. A phase is "✅" only
  when its acceptance criteria pass AND it has no open P0/P1 item.

## Related Projects

- [mini-infer](https://github.com/Deyu-Kong/mini-infer): C++/CUDA LLM inference engine from scratch (same "from scratch" philosophy)
- [runc](https://github.com/opencontainers/runc): The reference OCI runtime (tinybox is a simplified educational reimplementation)