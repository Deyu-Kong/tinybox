# Agent Guidelines for tinybox

> **Read [PLAN.md](docs/PLAN.md) first.** It is the authoritative, line-referenced
> audit of the current codebase (P0–P3 issues + remediation roadmap). The
> per-phase status in docs/PLAN.md supersedes any "✅" in older docs: several
> phases have open P0/P1 items and must not be considered complete until the
> corresponding milestone in docs/PLAN.md is tagged.

## Project Overview

tinybox is a from-scratch Rust implementation of a Linux sandbox runtime, similar to `runc` but simplified and focused on Agent workloads. It is built incrementally across 8 phases, each producing a runnable, verifiable deliverable.

> **⚠️ Security status (2026-08-16, post-M1): P0 isolation holes fixed,
> hardening incomplete.** Milestone M1 closed all four P0 items: the
> bridge/veth/NAT path was removed (Option A), the sandbox now always
> unshares `CLONE_NEWNET`, `clone` is seccomp-flag-masked to forbid
> `CLONE_NEW*`, and the capability bounding set is cleared. The `tinybox
> run` path is now a defensible isolation barrier. Remaining open items
> (P1 OCI field-honoring, P2 `/dev`/`/tmp`/`/sys` hardening, rootful) are
> correctness/depth issues, not escape holes. See [docs/PLAN.md](docs/PLAN.md).

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
- `aya` or `libbpf-rs` (eBPF — **research track R0 only**, not yet added)
  (fanotify and Landlock are reached via `libc` syscalls, no crate needed)

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
> ⚠️ **P0-1 / P0-2 (RESOLVED in M1, 2026-08-16):** the bridge path was
> removed (Option A) and the sandbox now always unshares `CLONE_NEWNET`.
> `--proxy` now provides real isolation: loopback-only netns + env vars.
> `scripts/test_phase7.sh` Test 3 asserts `--proxy` mode has no default
> route.

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
- Do NOT handle SELinux or AppArmor (seccomp + capabilities is the v1.0 LSM
  story; **Landlock is an explicit research-track addition** for the FS
  capability dimension — see [docs/VISION.md](docs/VISION.md) R1, not a
  contradiction of this rule)
- Do NOT implement user-mode networking (no TUN/TAP, no bridge)
- Do NOT use Docker, containerd, or runc libraries (this is a from-scratch project)
- Do NOT add Windows/macOS support (Linux-only)

> **Constraint violation in the tree (2026-08-16): RESOLVED.** Phase 11
> (`src/network.rs`) implemented a bridge + veth + NAT path that violated
> the "no TUN/TAP, no bridge" rule above and leaked to the host netns
> (P0-1). Milestone M1 (2026-08-16) took Option A: `src/network.rs` was
> deleted, `--network`/`-p`/`--publish` flags removed, and the sandbox now
> always unshares `CLONE_NEWNET`. The constraint and the tree are now
> consistent. (Network enforcement for the research track stays
> proxy-based — see [docs/VISION.md](docs/VISION.md); eBPF, if added in R0,
> is for audit/observation, not a bridge replacement.)

### What to prioritize
- **Correctness**: The sandbox must actually isolate. Leaking processes to the host is a bug.
- **Safety**: Default seccomp policy must prevent escape. `--dangerous` is opt-in.
- **Measurability**: Every optimization must be backed by a benchmark number.
- **Simplicity**: ~2000 lines total Rust for the **static isolation skeleton**
  (remediation track M0–M4). The research track (R0–R3 in
  [docs/VISION.md](docs/VISION.md)) is separately scoped and will exceed
  this budget. Favor readable code over clever abstractions.

> **Safety status (2026-08-16, post-M1):** the "default seccomp policy must
> prevent escape" priority is **now met** — P0-3 (escape primitives in the
> allow-list) and P0-4 (bounding set never cleared) are fixed. `clone` is
> flag-masked, escape syscalls are removed, and `PR_CAPBSET_DROP` is called
> for all 14 dangerous caps.

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

### 2026-08-16: Milestone M1 — P0 isolation holes closed
- **Decision**: P0-1 took **Option A** — `src/network.rs` (bridge + veth +
  NAT) was deleted entirely; `--network`/`-p`/`--publish` flags removed;
  `SandboxConfig.network`/`ports` fields removed. Rationale: the bridge path
  contradicted the 2026-08-02 "no bridge" decision and the "no TUN/TAP, no
  bridge" constraint, and its ordering bug leaked configuration onto the
  host netns. Restoring the documented proxy-only design also dropped the
  `ip`/`iptables` runtime dependency.
- **P0-2**: `child_main` now **always** inserts `CLONE_NEWNET` (the
  `proxy.is_none() && network.is_none()` gate was removed), so `--proxy`
  yields a loopback-only netns + env vars rather than sharing the host netns.
- **P0-3**: `clone` now carries a `SeccompCmpOp::MaskedEq(0x7E020000)` rule
  on arg0 that forbids any `CLONE_NEW*` bit (→ SIGSYS); `clone3` remains
  absent from the allow-list. Nine escape/interference syscalls were
  removed (`open_by_handle_at`, `process_vm_readv/writev`,
  `perf_event_open`, `ioprio_set`, `mbind`, `set_mempolicy`,
  `migrate_pages`, `move_pages`). `DANGEROUS_CAPS` grew 8 → 14 (added
  `CAP_DAC_READ_SEARCH`, `CAP_NET_RAW`, `CAP_AUDIT_WRITE`,
  `CAP_AUDIT_CONTROL`, `CAP_SETFCAP`, `CAP_SYSLOG`). Rule-building
  extracted into `build_rules()` for unit testing.
- **P0-4**: `drop_capabilities` now loops `DANGEROUS_CAPS` and calls
  `prctl(PR_CAPBSET_DROP, cap)` after the `capset` + ambient clear.
- **Verification**: `scripts/test_phase5.sh` Tests 5–7 (normal fork ok,
  `clone(CLONE_NEWUSER)` → SIGSYS, `CapBnd` cleared); `scripts/test_phase7.sh`
  Test 3 (`--proxy` no default route); two new seccomp unit tests; the
  `tests/phase5.rs` cap test extended to assert `CapBnd`. All acceptance
  gates green; `cargo test` (58 tests) + `cargo clippy -- -D warnings` clean.

### 2026-08-16: Vision reconciliation
- **Outcome**: [docs/VISION.md](docs/VISION.md) (research north star, R0–R3)
  was reconciled with [docs/PLAN.md](docs/PLAN.md) (remediation track, M0–M4).
  Dependency rule recorded in both: **R0 may run in parallel with M2; R1
  starts only after M2 closes**.
- **Re-prioritization**: P2-1 (`/dev`/`/tmp`/`/sys` hardening) was pulled
  forward from M3 into M2 — R0's acceptance (a sandboxed `pip install`
  producing an audit log) needs a working `/dev`/`/tmp`, so P2-1 is a
  prerequisite for the research track, not polish.
- **P1-2 resolved**: with `network.rs` deleted in M1, the `ip`/`iptables`
  silent-failure class no longer exists; marked RESOLVED in PLAN.md.
- **Open design questions surfaced** (not yet decided — flagged for R0/R1):
  - seccomp filters are **monotonic** (can only stack, never remove) →
    bidirectional dynamic grant cannot live in seccomp; network/FS/resource
    dimensions must carry the dynamic layer (eBPF maps, fanotify/Landlock,
    cgroup resize). `SECCOMP_RET_USER_NOTIF` is the only bidirectional
    syscall path, at a per-call latency cost.
  - proxy stays the **network enforcement** layer (L7, easy per-host
    allow/deny); eBPF egress is **audit/observation only**, not enforcement
    (keeps the "no bridge" constraint and avoids brittle TLS-SNI parsing).
  - Landlock is the candidate **FS enforcement** primitive (in-kernel, path
    policy); fanotify is the FS **audit** primitive.
- **LOC budget reframed**: the ~2000-line target now explicitly scopes only
  the static skeleton (M0–M4); the research track (R0–R3) is separately
  scoped and will exceed it.

## Related Projects

- [mini-infer](https://github.com/Deyu-Kong/mini-infer): C++/CUDA LLM inference engine from scratch (same "from scratch" philosophy)
- [runc](https://github.com/opencontainers/runc): The reference OCI runtime (tinybox is a simplified educational reimplementation)