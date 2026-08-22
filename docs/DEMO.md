# Customer Demo Guide

This demo tells one story: an Agent may download an approved dependency during
`install`, loses network access in `build`, cannot grant itself permissions, and
leaves structured evidence for every control-plane decision.

## Run the Demo

From the repository root:

```bash
cargo build
sudo ./scripts/demo_agent_capabilities.sh
```

Set `DEMO_PAUSE=0` for CI-style execution, or increase it while narrating:

```bash
sudo DEMO_PAUSE=2 ./scripts/demo_agent_capabilities.sh
```

The script uses only local fixtures and never depends on public network access.
It demonstrates:

1. An explicit, hashed capability descriptor selected outside the payload.
2. Exact host/port egress through the no-NIC broker.
3. Direct egress and access to the host control API failing from the payload.
4. CAS-protected phase transitions and replay rejection.
5. Immediate network revocation plus synchronized cgroup limits.
6. Bounded structured audit events explaining allow and deny decisions.
7. Host-side Landlock preventing secret reads and symlink escape.

## Suggested Narration

“tinybox is not merely another way to start a container. Its unit of control is
the Agent task. The user or orchestrator declares a capability ceiling; the
Agent cannot enlarge it. A legal phase transition can narrow active network and
resource permissions, and every decision carries the sandbox ID, policy hash,
phase, target, rule, and reason.”

Do not claim production readiness. The current runtime is rootful and does not
yet provide daemon authentication, persistence, rootless execution, complete
OCI support, or reversible dynamic filesystem/syscall grants.
