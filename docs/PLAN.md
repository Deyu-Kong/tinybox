# tinybox — Issue & Remediation Plan

This document is the authoritative, line-level audit of the tinybox codebase.
It records every defect discovered during the 2026-08-16 review, classifies it
by severity, and proposes a concrete fix. The README and AGENTS.md are kept in
sync with the status here; "Completed" in those docs refers only to features
that satisfy the acceptance criteria **and** have no open P0/P1 item below.

Conventions:
- `file:line` references use the tree as of commit `b73c7b1` (phase 13).
- Severity: **P0** = isolation/security broken · **P1** = correctness bug or
  documented feature not actually working · **P2** = claimed feature is shallow
  / incomplete · **P3** = polish, tech debt, non-blocking.
- Each item lists: **Problem · Location · Impact · Fix**.

---

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| P0 (isolation/security) | 4 | ✅ all resolved (M1 complete, 2026-08-16) |
| P1 (correctness/contradiction) | 5 | open |
| P2 (shallow feature) | 5 | open |
| P3 (polish) | 6 | open (one incidental fix landed) |

**P0 isolation holes are closed.** The four P0 items were resolved in
Milestone M1 (2026-08-16): the bridge/veth/NAT path was removed entirely
(Option A — `src/network.rs` deleted, `--network`/`-p` flags removed), the
sandbox now **always** unshares `CLONE_NEWNET` so `--proxy` is real
isolation (loopback-only + env vars), the seccomp allow-list had its escape
primitives removed and `clone` restricted to forbid `CLONE_NEW*` flags, and
the capability bounding set is now cleared via `PR_CAPBSET_DROP`.

The process-isolation skeleton (namespaces + overlayfs + cgroups + seccomp +
caps) is now a defensible barrier for the `tinybox run` path. Remaining open
items (P1 OCI field honoring, P2 rootfs/device hardening, etc.) are
correctness/depth issues, not escape holes. tinybox is still **rootful** and
lacks `/dev`/`/tmp`/`/sys` hardening, so it is not yet a production-grade
boundary — but it no longer leaks to the host.

---

## P0 — Isolation / Security ✅ RESOLVED (M1)

> All four P0 items below were fixed on 2026-08-16 (commit after `0531141`).
> The text is retained as the historical record of the defect + fix.

### P0-1 `--network bridge` configures the host network instead of the sandbox ✅
- **Problem**: When `--network` is set, `child_main` does **not** add
  `CLONE_NEWNET` to the `unshare` flags. The parent then calls
  `network::move_veth_to_ns(child_pid)` while the child is still blocked on the
  sync pipe — i.e. still in the host netns. The veth therefore stays in the host
  netns, and `configure_container_network` assigns `172.20.x.y` to an interface
  and installs a default route **on the host**. This is network leakage, not
  isolation.
- **Location**: `src/sandbox.rs:141-143` (NEWNET gated on `proxy.is_none() &&
  network.is_none()`), `src/sandbox.rs:98` (parent calls `move_veth_to_ns`),
  `src/network.rs:113,122`.
- **Impact**: Any `--network bridge` sandbox mutates host routes/interfaces.
  Potentially disrupts host connectivity and is a privilege boundary violation.
- **Fix (two options, decision required — see Decision Log 2026-08-16)**:
  - **Option A (align with AGENTS.md)**: delete `network.rs` bridge/NAT path
    entirely; keep proxy-only. `--network` becomes a no-op alias for the
    default (no NEWNET) or is removed. Restores documented design, drops
    ~187 LOC, removes the `ip`/`iptables` binary dependency.
  - **Option B (keep bridge, fix it)**: in `child_main`, always unshare
    `CLONE_NEWNET` when `--network` is set; move the `move_veth_to_ns` call to
    **after** the child has unshared (re-order the pipe sync so the parent
    moves the veth once the child's netns exists). Then `configure_container_network`
    runs inside the child post-unshare. Add a regression test asserting the host
    route table is unchanged after `--network bridge` run.
- **Recommendation**: Option A for v0.x (matches the documented "proxy-based,
  no bridge" decision and the "no TUN/TAP, no bridge" constraint). Revisit
  bridge as a v1.0 opt-in once seccomp + cap story is solid.
- **Resolution (2026-08-16, Option A taken)**: `src/network.rs` deleted
  entirely; `--network` and `-p`/`--publish` flags removed from `main.rs`;
  `SandboxConfig.network`/`ports` fields removed; the bridge/veth/port-mapping
  blocks in `sandbox.rs` removed; `scripts/test_phase11.sh` deleted (it tested
  the removed bridge). The `ip`/`iptables` binary dependency is gone. P0-2
  (below) covers the resulting `--proxy` semantics.

### P0-2 `--proxy` provides no isolation ✅
- **Problem**: `--proxy <URL>` only pushes `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`
  env vars. No `CLONE_NEWNET` is created when `--proxy` is set, so the sandbox
  shares the host netns; any binary that ignores the proxy env bypasses it.
- **Location**: `src/sandbox.rs:141-143`, `src/sandbox.rs:206` (`effective_environment`).
- **Impact**: The Phase 7 acceptance claim "`--proxy ... wget` succeeds" is
  vacuously true — it succeeds because the sandbox has full host network, not
  because traffic is proxied. Reverse claim ("`ping 8.8.8.8` → unreachable")
  only holds when **neither** `--proxy` nor `--network` is passed.
- **Fix**: Always unshare `CLONE_NEWNET`. For `--proxy` mode, keep the netns
  empty (only `lo`) and set the env vars; for `--network bridge`, run the veth
  setup inside the child. Make the Phase 7 acceptance test assert that
  `ping 8.8.8.8` fails **with** `--proxy` set (only loopback reachable).
- **Resolution (2026-08-16)**: `child_main` now always inserts `CLONE_NEWNET`
  (the `proxy.is_none() && network.is_none()` gate was removed). `--proxy`
  therefore yields a loopback-only netns + env vars. Regression:
  `scripts/test_phase7.sh` Test 3 asserts `--proxy` mode has no default route.

### P0-3 seccomp allow-list contains escape primitives ✅
- **Problem**: The allow-list permits syscalls that are well-known container
  escape / host-interference primitives:
  - `clone` (no argument filtering) — a sandboxed process can
    `clone(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWNS)` to build fresh
    namespaces and sidestep the missing `unshare`/`setns`/`pivot_root` blocks.
  - `open_by_handle_at` — combined with the still-present
    `CAP_DAC_READ_SEARCH`, this is a classic container-escape primitive.
  - `process_vm_readv` / `process_vm_writev` — cross-process memory R/W.
  - `perf_event_open`, `ioprio_set`, `mbind`, `set_mempolicy`,
    `migrate_pages`, `move_pages` — host resource interference / side channels.
- **Location**: `src/seccomp.rs:163,349,353,357-358` and the cap drop list at
  `src/seccomp.rs:20-38`.
- **Impact**: Violates AGENTS.md "default seccomp policy must prevent escape."
- **Fix**:
  1. Add `SeccompRule` argument conditions on `clone` to allow only
     `CLONE_VFORK | SIGCHLD`-style flags (or replace `clone` with `clone3`
     restricted to `exit_signal`). Block `CLONE_NEW*` bits.
  2. Remove `open_by_handle_at`, `process_vm_readv`, `process_vm_writev`,
     `perf_event_open`, `ioprio_set`, `mbind`, `set_mempolicy`,
     `migrate_pages`, `move_pages` from the allow-list.
  3. Add `CAP_DAC_READ_SEARCH`, `CAP_NET_RAW`, `CAP_SYSLOG`,
     `CAP_AUDIT_*`, `CAP_SETFCAP` to the dropped set.
  4. Add a `// SAFETY:` note documenting the residual risk and the
     `--dangerous` escape hatch.
- **Resolution (2026-08-16)**: all four sub-fixes landed in `src/seccomp.rs`.
  `clone` now carries a `SeccompCmpOp::MaskedEq(0x7E020000)` rule on arg0 so
  any `CLONE_NEW*` bit → SIGSYS; `clone3` remains absent from the allow-list.
  The nine escape/interference syscalls were removed. `DANGEROUS_CAPS` grew
  from 8 → 14 (added `CAP_DAC_READ_SEARCH`, `CAP_NET_RAW`, `CAP_AUDIT_WRITE`,
  `CAP_AUDIT_CONTROL`, `CAP_SETFCAP`, `CAP_SYSLOG`). Rule-building was
  extracted into `build_rules()` for unit testing. Regressions:
  `scripts/test_phase5.sh` Test 6 (`clone(CLONE_NEWUSER)` → SIGSYS 159) and
  two new seccomp unit tests (`test_clone_rule_blocks_namespace_flags`,
  `test_escape_syscalls_excluded`). Normal fork (`clone(SIGCHLD)`) still
  passes (Test 5).

### P0-4 capability bounding set is never cleared ✅
- **Problem**: `drop_capabilities` clears effective/permitted/inheritable/
  ambient sets but never calls `prctl(PR_CAPBSET_DROP, ...)`. A setuid binary
  exec'd inside the sandbox re-acquires dropped caps from the bounding set on
  `execve`.
- **Location**: `src/seccomp.rs:40-95` (no `PR_CAPBSET_DROP`).
- **Impact**: A sandbox that exec's a setuid-root binary regains `CAP_SYS_ADMIN`
  etc., defeating the cap drop.
- **Fix**: After clearing effective/permitted/inheritable, iterate the dropped
  cap list and call `prctl(PR_CAPBSET_DROP, cap)` for each. Add a unit test
  that reads `/proc/self/status` `CapBnd` and asserts the dangerous caps are
  absent.
- **Resolution (2026-08-16)**: `drop_capabilities` now loops `DANGEROUS_CAPS`
  and calls `libc::prctl(PR_CAPBSET_DROP, cap, ...)` for each. Regressions:
  `scripts/test_phase5.sh` Test 7 asserts `CapBnd` has `CAP_SYS_ADMIN` (bit
  21) cleared; `tests/phase5.rs::test_capabilities_dropped` extended to also
  assert `CapBnd` for `CAP_SYS_ADMIN` + `CAP_NET_ADMIN`.

---

## P1 — Correctness / Contradictions

### P1-1 OCI support ignores `linux.namespaces` (and almost everything else)
- **Problem**: `OciConfig` deserializes only `process.{args,env}` and
  `root.path`. The Phase 6 acceptance config includes `linux.namespaces` —
  tinybox silently ignores them and always creates the same namespace set.
  `root.readonly`, `mounts`, `process.{cwd,user,capabilities}`,
  `linux.{resources,seccomp,sysctl,cgroupsPath}` are all dropped.
- **Location**: `src/oci.rs:7-30`.
- **Impact**: "Phase 6 ✅ OCI Bundle support" is misleading — only ~3 of the
  claimed "core 10" fields are honored. An OCI bundle that relies on a *subset*
  of namespaces (e.g. only `pid` + `mount`) will get a different (broader)
  isolation set than requested.
- **Fix**: Extend `OciConfig` to at least honor:
  `root.readonly` (apply `MS_RDONLY` to the overlay),
  `process.cwd`, `process.user.{uid,gid}`,
  `hostname`, and `linux.namespaces` (drive which `CLONE_NEW*` flags to set in
  `child_main`). Document the remaining unsupported fields as explicitly
  ignored. Update the Phase 6 acceptance test to assert a namespace-restricted
  config actually restricts namespaces.

### P1-2 `ip`/`iptables` non-zero exit is silently treated as success
- **Problem**: Throughout `network.rs`, `.status().context(...)?` only
  propagates the `io::Error` from spawning the command; a non-zero exit of
  `ip`/`iptables` is swallowed and treated as success.
- **Location**: `src/network.rs:40,46,50,67,76,86,97,107,116,126,...`.
- **Impact**: On a host without `ip`/`iptables`, or on any rule-insert
  failure, tinybox reports success while networking is broken. Coupled with
  P0-1, the failure is silent and dangerous.
- **Fix**: Wrap each command in a helper `fn run(cmd) -> Result<()>` that
  checks `status.success()` and returns `anyhow::bail!` with stderr otherwise.
  (Moot if P0-1 Option A is taken — `network.rs` is deleted.)

### P1-3 daemon conflates failed and completed sandboxes
- **Problem**: `create` sets `status="completed"` and `exit_code = result.ok()`
  on both success and error; failures leave `exit_code=None` but still count as
  "completed". `metrics` computes `completed = total - running`, so errored
  sandboxes inflate the completed counter.
- **Location**: `src/daemon.rs:105` (`exit_code = result.ok()`), `:143`.
- **Impact**: `/metrics` and `GET /api/sandboxes` misreport health; an operator
  cannot distinguish a crashed sandbox from a successful one.
- **Fix**: Introduce `status` values `{running, completed, failed}`. On error
  set `status="failed"`, capture `exit_code` and an `error` string. Expose
  `tinybox_sandboxes_failed` as a separate Prometheus counter.

### P1-4 daemon `CreateRequest` cannot set most sandbox options
- **Problem**: The HTTP `CreateRequest` only accepts `rootfs`, `command`,
  `memory_limit_mb`, `proxy`. The `SandboxConfig` it builds hard-codes cpus,
  pids_limit, volumes, ports, network, hostname, env, image, oci, dangerous.
- **Location**: `src/daemon.rs:59-95`.
- **Impact**: The API cannot exercise the features the CLI exposes; Phase 8
  acceptance only verifies the minimal `sleep 30` case.
- **Fix**: Extend `CreateRequest` with optional `cpus`, `pids_limit`,
  `volumes`, `ports`, `network`, `hostname`, `env`, `image`, `oci`. Reject
  `dangerous=true` over the API (or require an explicit opt-in flag) to avoid
  remote sandbox-disable footgun.

### P1-5 `exec.rs` is a 23-line `nsenter` wrapper with gaps
- **Problem**: `exec_in_container` shells out to `nsenter -t <pid> -m -u -n -p`,
  missing `-i` (IPC), `-U` (user), `-C` (cgroup). No TTY allocation, no
  `--cwd`/`--env`/`--user`, no validation that `<pid>` is a tinybox sandbox.
- **Location**: `src/exec.rs:4-16`.
- **Impact**: Exec'd processes don't share IPC/user/cgroup namespaces;
  interactive shells have no controlling terminal; any host PID can be
  targeted (privilege footgun if `exec` is ever wired into the daemon API).
- **Fix**: Replace the `nsenter` shell-out with `nix::sched::setns` calls for
  each of the target's namespaces (read `/proc/<pid>/ns/*` symlinks). Add
  `-i/-U/-C` equivalents. Track sandbox PIDs in `daemon::AppState` and reject
  PIDs not in that set. Add `--cwd`/`--env`/`--user` flags.

---

## P2 — Shallow / Incomplete Features

### P2-1 rootfs missing `/dev`, `/tmp`, `sysfs`; only `/proc` mounted
- **Location**: `src/sandbox.rs:217` (`mount_proc`), `src/rootfs.rs`.
- **Fix**: After pivot, mount `tmpfs` on `/dev`, create `/dev/pts`, `/dev/shm`,
  `/dev/mqueue`; bind `/dev/null`, `/dev/zero`, `/dev/urandom`, `/dev/tty` from
  host; mount `tmpfs` on `/tmp` with a size cap; mount `sysfs` on `/sys` (read-only).
  Honor OCI `root.readonly` by applying `MS_RDONLY` to the overlay.

### P2-2 cgroup: no v2 validation, no controller enabling, swap hardcoded
- **Location**: `src/cgroup.rs:23,35,38-51`.
- **Fix**: Validate `/sys/fs/cgroup/cgroup.controllers` exists (v2). Write
  `+memory +cpu +pids` to `cgroup.subtree_control` at the parent where needed.
  Make `swap.max` configurable (default 0). Add `io.max` and `cpu.weight`.

### P2-3 image storage: no content addressing, no layering, no metadata
- **Location**: `src/image.rs`.
- **Fix**: Store images as `<store>/<sha256>/` with alias symlinks
  `<store>/aliases/<name> -> ../../<sha256>`. Support layered extraction
  (whiteouts). Write a `metadata.json` per image (created, size, parent, labels).

### P2-4 registry pull: in-memory blobs, no config, no digest verify, Docker Hub only
- **Location**: `src/registry.rs:85-100,103,112`.
- **Fix**: Stream blobs to a temp file (avoid OOM). Fetch the config blob and
  surface `Cmd`/`Entrypoint`/`Env`/`WorkingDir` as default command when the
  user omits `command`. Verify `docker-content-digest` against the manifest.
  Support `registry-host[:port]/repo:tag` parsing (split on the first `/`).
  Add HTTP timeouts and retries.

### P2-5 daemon: no persistence, no auth, no streaming/log endpoints
- **Location**: `src/daemon.rs`.
- **Fix**: Persist `AppState` to `$TINYBOX_STATE_DIR/sandboxes.json`. Add
  `GET /api/sandboxes/:id/logs` (stream stdout/stderr captured to a file).
  Add bearer-token auth behind a `--auth-token` flag. Add `POST
  /api/sandboxes/:id/exec` once P1-5 is fixed. Add graceful shutdown on SIGTERM.

---

## P3 — Polish / Tech Debt

### P3-1 `tracing` approved but unused; logging is `eprintln!`
- **Fix**: Replace `eprintln!` with `tracing::{info,warn,error}` + a
  `tracing_subscriber::fmt` init in `main`. Gate verbose output behind `-v`.

### P3-2 `signal_to_int` returns 0 for unmapped signals
- **Location**: `src/sandbox.rs:266-284`.
- **Fix**: Default to `128 + signum` for any unmapped signal; reserve a
  sentinel (`255`) for "killed by unknown signal".

### P3-3 `parse_port_spec` only supports `host:container` TCP
- **Location**: `src/sandbox.rs:288`.
- **Fix**: Support `ip:host:container`, `port-range`, and `udp` suffix
  (`8080:80/udp`). Moot under P0-1 Option A.

### P3-4 `mount_proc` swallows `create_dir_all` errors via `.ok()`
- **Location**: `src/sandbox.rs:217`.
- **Fix**: Propagate the error; only `.ok()` the mount if `/proc` is already
  mounted.

### P3-5 double cgroup cleanup (`Drop` + manual `drop(cg)`)
- **Location**: `src/cgroup.rs:73-80`, `src/sandbox.rs:117`.
- **Fix**: Remove the manual `drop`; rely on `Drop`.

### P3-6 test hygiene: shared `TINYBOX_IMAGE_DIR` env mutated across tests
- **Location**: `src/image.rs` tests.
- **Fix**: Use `tempfile::TempDir` per test, set the env in-process only.

---

## Remediation Roadmap

Ordered so that each milestone leaves the tree in a defensible state. Target
commits follow the existing `phase N:` / `fix:` convention; tag at each
milestone.

### Milestone M0 — "Honest baseline" (no behavior change) ✅
1. Add this `PLAN.md`. ✅ (commit `0531141`)
2. Correct README badges/status and AGENTS decision log to reflect the real
   state. ✅ (commit `0531141`)
3. Add a `WARNING: not a security boundary` notice to README and `--help`. ✅

### Milestone M1 — Close P0 isolation holes ✅ (2026-08-16)
1. ✅ P0-1 Option A: `src/network.rs` deleted; `--network`/`-p` flags removed;
   `ip`/`iptables` runtime dependency gone.
2. ✅ P0-2: `child_main` always unshares `CLONE_NEWNET`; `--proxy` =
   loopback-only + env vars.
3. ✅ P0-3: `clone` restricted via `MaskedEq(0x7E020000)` (forbids `CLONE_NEW*`);
   nine escape/interference syscalls removed; `DANGEROUS_CAPS` 8 → 14.
4. ✅ P0-4: `drop_capabilities` clears the bounding set via `PR_CAPBSET_DROP`.
5. ✅ Regression tests: `test_phase5.sh` Tests 5–7 (normal fork ok,
   `clone(CLONE_NEWUSER)` → SIGSYS, `CapBnd` cleared); `test_phase7.sh`
   Test 3 (`--proxy` no default route); seccomp unit tests for clone rule +
   excluded syscalls. All acceptance gates green.

### Milestone M2 — Make claimed features actually work
1. P1-1: honor OCI `linux.namespaces`, `root.readonly`, `process.cwd`,
   `process.user`.
2. P1-2: (skipped if M1 took P0-1 Option A.)
3. P1-3: daemon status `{running,completed,failed}` + failed metrics counter.
4. P1-4: extend `CreateRequest`; reject remote `dangerous`.
5. P1-5: `exec` via `setns`, namespace-complete, PID-validated, with TTY.

### Milestone M3 — Depth where it matters
1. P2-1: full `/dev`, `/tmp`, `/sys` setup.
2. P2-2: cgroup v2 validation + controller enabling.
3. P2-3/P2-4: content-addressed images + registry config blob fetch + streaming.
4. P2-5: daemon persistence + logs + auth + exec endpoint.

### Milestone M4 — Polish
1. P3-1 through P3-6.

### Stretch (explicitly out of scope for v1.0)
- rootless operation via `CLONE_NEWUSER` + uid mapping
- cgroup namespace
- UDP port mapping / hairpin NAT (only if bridge is retained)
- multi-host anything

---

## Verification gates (must pass before tagging a milestone)

- `cargo test && cargo clippy -- -D warnings` (existing gate).
- New: `scripts/test_phase7.sh` asserts host route table unchanged after a
  `--proxy` run and after a (retained) `--network` run.
- New: `scripts/test_phase5.sh` asserts `clone(CLONE_NEWUSER)` returns `EPERM`
  inside a sandbox.
- New: `scripts/test_phase6.sh` asserts an OCI config requesting only
  `{pid,mount}` namespaces actually restricts to those.
- New: `scripts/test_phase8.sh` asserts `/metrics` reports
  `tinybox_sandboxes_failed` after a deliberately-crashing sandbox.

---

## Status legend for README/AGENTS

When updating per-phase status, use:
- ✅ **works** — meets acceptance, no open P0/P1 item.
- ⚠️ **partial** — runs but has an open P1/P2 item (see PLAN.md).
- ❌ **broken** — has an open P0 item or fails acceptance.

Current per-phase status (post-M1, 2026-08-16):

| Phase | Feature | Status | Open items |
|-------|---------|--------|------------|
| 1 | skeleton + CLI + exec | ✅ | — |
| 2 | namespaces (pid/mount/uts/net) | ✅ | — (NEWNET now always unshared) |
| 3 | overlayfs + pivot_root | ⚠️ | P2-1 (no /dev, /tmp, /sys) |
| 4 | cgroup limits | ⚠️ | P2-2 (no v2 validation, swap hardcoded) |
| 5 | seccomp + caps | ✅ | — (P0-3, P0-4 fixed in M1) |
| 6 | OCI bundle | ❌ | P1-1 (namespaces ignored) |
| 7 | network (proxy-only) | ✅ | — (P0-1, P0-2 fixed in M1; bridge removed) |
| 8 | daemon API | ⚠️ | P1-3, P1-4, P2-5 |
| 9 | local images | ⚠️ | P2-3 |
| 10 | registry pull | ⚠️ | P2-4 |
| 11 | ~~network bridge~~ | 🗑 removed | removed in M1 (Option A); was P0-1 |
| 12 | volumes | ✅ | — |
| 13 | exec | ⚠️ | P1-5 |
