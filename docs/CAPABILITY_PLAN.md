# tinybox Agent 能力管理实施计划

> **ACTIVE，2026-08-17。** 本文件指导后续模型实现 tinybox 的核心差异化
> 能力：由用户/编排器授权、runtime 强制、Agent 无法自行扩权的任务级权限。
> 当前缺陷听 [PLAN.md](PLAN.md)，实施顺序听本文，长期方向听
> [VISION.md](VISION.md)。

## 1. 目标与完成定义

```text
用户/编排器声明 CapabilityDescriptor
        → tinybox 校验、规范化并编译
        → 内核原语与 host broker 强制
        → 允许行为成功，越权行为拒绝并审计
        → 编排器按合法 phase transition 调整有效权限
```

v1 只有同时满足以下条件才算完成：

1. CLI `tinybox run --policy policy.json -- ...` 与 daemon 使用同一 schema。
2. FS、网络、CPU、内存、pids 均来自策略；未声明能力默认拒绝。
3. payload 不能读取控制凭据、修改策略或给自己 grant。
4. 网络 allowlist 有真实数据通路，不把 proxy 环境变量冒充强制。
5. 每次拒绝可关联 sandbox ID、policy hash、phase、目标和原因。
6. 同一任务可从 `install` 转到 `build/test`，撤销安装网络并调整资源。
7. root 验收、非特权测试、fmt、clippy 分开报告；skip 不等于 pass。

## 2. 范围、非目标与威胁边界

v1 使用模式 C：Agent runtime 留在 host，高风险 bash/python/compiler/test
进入 per-task sandbox。用户/编排器是授权主体，Agent 和 payload 均是对抗方。

- FS：mount 视图 + Landlock 形成不可放宽的静态 ceiling。
- 网络：无 NIC、无 bridge；sandbox local proxy 经 AF_UNIX 连接 host broker。
- syscall：静态 seccomp 底盘，可继续收紧但不承诺动态放宽。
- 资源：cgroup v2，在初始 ceiling 内动态 resize。
- phase v1：显式 marker + 状态机；纯行为推断留到后续研究。

不做完整 OCI、rootless、GPU、多节点、TUN/TAP、bridge、透明 UDP、TLS SNI
策略或预热池。执行沙箱也不承诺阻止通过合法 LLM API 外泄合法可读数据。

## 3. 安全不变量

- **Fail closed**：未知字段/namespace、无效路径、broker 错误均拒绝。
- **No self-grant**：控制 FD 和 phase API 凭据不进入 payload。
- **Ceiling first**：动态 phase 永不超过任务初始授权预算。
- **Private mounts**：rootfs、volume、`/proc`、`/dev` 只在私有 mount ns 初始化。
- **No direct egress**：payload 无路由；允许流量也只能经过 broker。
- **Stable identity**：sandbox ID、cgroup、PID、policy hash 绑定，不能只匹配名称。
- **Bounded audit**：日志有界且不记录 token、body、文件内容或完整环境变量。
- **Honest status**：setup failure、policy denial、payload exit code 分开表示。

## 4. 目标代码结构与 schema

```text
src/policy.rs       schema、校验、规范化、hash
src/landlock.rs     FS ceiling
src/broker.rs       host 网络 broker 与 allowlist
src/proxy.rs        sandbox 内 HTTP/CONNECT helper
src/audit.rs        有界事件缓冲与 API
src/phase.rs        phase 状态机
tests/policy.rs     无特权 schema/状态机测试
tests/capability.rs root 集成测试
scripts/test_capability.sh
scripts/attacks/
```

建议的第一版数据模型：

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    pub version: u32,
    pub filesystem: Vec<FsRule>,
    pub network: Vec<NetworkRule>,
    pub resources: ResourcePolicy,
    pub phases: Vec<PhasePolicy>,
}
```

```json
{
  "version": 1,
  "filesystem": [
    {"path": "/workspace", "access": "read"},
    {"path": "/workspace/output", "access": "read_write"},
    {"path": "/tmp", "access": "read_write"}
  ],
  "network": [],
  "resources": {"memory_bytes": 536870912, "cpus": 1.0, "pids": 50},
  "phases": []
}
```

路径必须是 sandbox 视图中的规范绝对路径；拒绝相对路径、`..`、NUL 和符号
链接逃逸。网络规则使用规范 host + port，不接受 URL。规范化 JSON 计算稳定
SHA-256；若需新增 hash crate，先记录最小依赖理由，不能伪造 hash。

## 5. C0——可信底座

**目的：关闭 PLAN A1、A2、A3、A5、A6，并保持 A4 明确未实现；A4 的真实
网络数据通路由 C3 关闭。C0 未完成前不得实现动态授权。**

### C0.1 Namespace/OCI fail-closed

- 用 enum 解析 namespace；未知类型报错，不回退默认集合。
- rootfs、volume、特殊 FS 强制要求 mount namespace。
- OCI `user` namespace 在 uid mapping 未实现前明确拒绝，不能静默忽略。
- 默认路径仍创建 PID/Mount/UTS/Net namespace。

验收：`{pid}` + rootfs 在 fork 前失败；未知 namespace 失败；`{pid,mount}`
执行前后宿主 mountinfo 不变。

### C0.2 Child setup 错误协议

- 增加 `CLOEXEC` setup pipe。
- namespace、mount、setuid、cwd、seccomp、exec 失败写结构化错误。
- exec 成功自动关闭 pipe，parent 才进入 payload wait。
- daemon 状态使用 `setup_failed`；普通 `exit 1` 仍是 completed + code 1。

验收：不存在 executable、非法 cwd、mount failure 是 `setup_failed`；
`sh -c 'exit 1'` 是正常完成。

### C0.3 Rootfs/volume fail-closed

- 移除关键 mount、目录创建和 device bind 上的 `.ok()`。
- 明确设备节点、devpts、shm、sys 的必需与可选级别。
- `/dev/mqueue` 要么实现，要么从声称能力中删除。
- 只读 bind 在初次 bind 后使用
  `MS_REMOUNT|MS_BIND|MS_RDONLY|MS_NOSUID|MS_NODEV`。
- 根据 source 类型创建 target 文件或目录；拒绝不存在 source。

### C0.4 测试与平台检查

- 特权测试 skip 必须显式；CI 拆成 unprivileged 与 root job。
- 先做独立纯格式化变更，再把 `cargo fmt -- --check` 纳入门禁。
- 检测 `cgroup.controllers` 和所需 controller；不只检测目录存在。
- CLI `--help` 增加 experimental/rootful 警告。

**完成门：** A1/A2/A3/A5/A6 有回归测试；A4 由 C3 关闭。在此之前文档
不得声称 proxy 可用或 M2 已关闭。

## 6. C1——静态 CapabilityDescriptor

### C1.1 Schema 和配置合并

- 实现 `policy.rs`、版本、`deny_unknown_fields`、语义校验和规范化。
- CLI/API 走同一 parser；API 返回 policy hash。
- policy 是 ceiling；CLI/OCI 只能进一步收紧，不能放宽。
- `--dangerous` 与 `--policy` 互斥；daemon 始终拒绝 dangerous。
- 无 policy 保留 legacy 兼容，但输出明确警告，不宣称具备 Agent 权限管理。

### C1.2 资源编译

- memory/cpus/pids 编译到 `CgroupConfig`。
- policy 模式下“未声明”使用文档化安全默认值或拒绝，不能等同 unlimited。
- 保存初始 ceiling 供 C5 校验。

### C1.3 网络默认拒绝

- `network: []`：私有 netns、无 route、无 broker FD。
- 非空规则在 C3 未可用前返回 unsupported，不能只注入 proxy env 后运行。

验收：离线分析 workspace、资源限制、无网络、hash 可查；未知字段、非法路径、
越过 ceiling 的 CLI 参数全部失败。

## 7. C2——文件系统能力 ceiling

- rootfs 默认只读，只把声明路径按最小权限 bind 进 sandbox。
- `/tmp` 是容量受限 tmpfs；宿主 home、SSH、Docker/daemon socket 不可见。
- 检测 Landlock ABI，按内核支持位编译规则。
- exec 前创建 ruleset、添加 allow rule、设置 `NO_NEW_PRIVS` 并 restrict self。
- 内核不支持所需 ABI 时 policy 模式 fail closed；不得默认静默降级。
- Landlock 是生命周期 ceiling，不承担动态放宽。

验收：允许 output 写成功；读 `/etc/shadow`/`~/.ssh`、越权 workspace、symlink、
rename/link 绕过失败。这里只保证 sandbox payload；host Agent 的 Landlock launcher
留给 C6，不能提前宣称模式 C 两层强制完成。

## 8. C3——无 NIC 网络 broker

```text
payload → 127.0.0.1:proxy（sandbox helper）
        → 预先建立的 AF_UNIX channel
        → host broker → DNS/connect → approved host:port
```

- helper 在私有 netns 拉起 loopback，只接受本 sandbox 连接。
- Unix channel 在创建 namespace 前建立；payload 不得到控制通道。
- host broker 在 host netns 解析 DNS、检查 allowlist、连接并审计。
- 第一版只支持 HTTP 与 CONNECT TCP；忽略 proxy env 的程序仍无直接 route。
- host 规范化为小写并去终止点；拒绝 userinfo、端口歧义和非法 IP 表示。
- 默认拒绝 loopback、private、link-local、metadata IP，除非产品明确另行设计。
- DNS rebinding 测试必须覆盖解析结果与实际连接目标的一致性。
- 废弃“任意 `--proxy URL` 就获得网络”的语义；upstream 只能由 broker 使用。

验收：允许 fixture 成功；未允许 host、字面 IP、metadata、直接 socket 失败；
测试不得依赖公网。网络 allow/deny 的结构化事件由紧随其后的 C4 接入。

**完成（2026-08-17）：** helper 在私有 netns 拉起 loopback，仅支持 CONNECT；
host broker 精确匹配规范化 host/port、解析并连接目标，再用 `SCM_RIGHTS`
回传已连接 socket。除测试专用的显式 `localhost` 规则外，private、loopback、
link-local 与 metadata 地址均拒绝；payload 直连无路由。`test_c3.sh` 使用本地
fixture 覆盖允许、未允许和直连失败。统一审计仍属于 C4，未提前宣称完成。

## 9. C4——统一审计

事件最少包含：timestamp、sandbox ID、policy hash、phase、source、decision、
capability、target、rule ID、reason。不得包含请求 body、token 或文件内容。

- 每 sandbox 使用有界 ring buffer并暴露 dropped-events 计数。
- API：`GET /api/sandboxes/:id/audit` 与 `/audit/summary`；SSE 可后置。
- 先记录 runtime、Landlock setup、broker、cgroup 决策。
- fanotify 是后续 FS audit；seccomp `RET_LOG` 先做可行性实验。
- eBPF 只有现有事件无法回答研究问题时引入，需独立依赖决策。

**完成门：** 即使没有 eBPF，也能用结构化证据解释一次权限允许或拒绝。

**完成（2026-08-17）：** 每 sandbox 使用容量 1024 的 ring buffer，覆盖时增加
`dropped_events`；事件包含计划要求的身份、phase、来源、决策、能力、target、
rule ID 与 reason。daemon 暴露 `/audit` 和 `/audit/summary`，runtime、Landlock、
cgroup 与 broker 均已接入。C4 不记录 body、token、文件内容或完整环境变量；
fanotify、seccomp `RET_LOG` 与 SSE 明确保留为后续增强。

## 10. C5——Phase-scoped 动态权限

- 顶层 descriptor 是不可变 ceiling；每个 phase 是它的子集。
- phase 声明合法 next；请求携带 expected generation，以 CAS 防并发与重放。
- 控制面提供 `POST /api/sandboxes/:id/phase`，payload 不可访问。
- 网络：原子更新 broker allowlist。
- 资源：更新 cgroup，但不得超过 ceiling。
- FS：Landlock 不变；v1 只允许不变或单调收紧。双向 FS 仲裁需后续外部
  fanotify permission mediator，未实现前不得声称支持。
- syscall：seccomp 只能叠加收紧，不能撤销后恢复。

验收：同一 sandbox 在 install phase 可访问本地 package fixture；切换 build 后
立即拒绝；资源同步变化。伪造 marker、重放 generation、越过 ceiling 均失败并审计。

**完成（2026-08-17）：** descriptor 编译时校验 phase 名、next graph、网络与资源
均不越过顶层 ceiling；daemon 以首个 phase 启动，并通过带
`expected_generation` 的 `/phase` API 执行 CAS 转换。成功转换更新共享 broker
allowlist、cgroup memory/CPU/pids 与审计 phase；伪造 next、重放 generation 和
运行结束后的请求 fail closed 并审计。Landlock ceiling 在 v1 中保持不变；双向
FS 授权和 syscall 放宽仍明确不支持。控制面仅存在宿主 daemon，私有 netns payload
无到达路径；daemon 本身的多租户认证仍是独立生产化工作，不能宣称已解决。

## 11. C6——Agent 集成与研究证据

- 提供 wrapper，将 bash/python/compiler/test 路由到 tinybox，同时保持
  stdout/stderr/exit code 语义。
- policy 由用户在任务开始时选择，模型输出不能修改策略或控制 API。
- 单独实现 host Agent Landlock launcher，并记录对 edit/git/LLM SDK 的影响。
- 固定三个 workload：离线分析；安装→构建→测试；主动越权攻击。
- 攻击覆盖 SSH、metadata、direct egress、symlink、fork bomb、self-grant。
- 量化 cold-start p50/p95、RSS、任务延迟、policy switch、audit overhead。
- 对比 native 和 runc；只有环境可复现时再加入 Firecracker。

## 12. 测试矩阵

| 层 | 内容 | Root |
|---|---|---|
| schema | unknown field、路径、host、ceiling、phase graph | 否 |
| 状态机 | transition、generation、重放、并发 | 否 |
| broker | parser、host/port、DNS/IP 分类 | 否 |
| sandbox | namespace、mount、Landlock、cgroup、helper | 是 |
| API | setup_failed、hash、phase、audit | 是 |
| attacks | symlink、egress、metadata、self-grant、fork bomb | 是 |
| benchmark | cold/warm、RSS、switch | 环境专属 |

网络测试使用本地 DNS/HTTP fixture，且必须证明 payload 不能绕过 broker 直接访问
fixture。任何 skip 都要单独计数并使“完整验收”门失败。

## 13. 后续模型执行规则

1. 开工前读 `AGENTS.md`、`PLAN.md` 和本文当前里程碑，并检查 `git status`。
2. 一次只做一个 C 子项；不得顺手加入后续 eBPF、模型推断或预热池。
3. 先写失败回归测试，再改实现；安全路径必须 fail closed。
4. 保留用户和前序模型修改；不自动 commit、tag 或 push。
5. 不新增依赖，除非 libc/nix/标准库不能合理完成，并记录原因。
6. 同步更新 PLAN 状态、README 功能表及本文状态；代码存在不等于完成。
7. 报告实际执行的命令；未 root 验收必须明确写“未执行”。

交付报告固定包含：Outcome、Not implemented、Security invariants、Validation、
Documentation。每个 tag 只有 root 验收证据存在且用户明确授权后才能创建。

## 14. 里程碑与当前状态

| Milestone | 状态 | 依赖/阻塞 |
|---|---|---|
| C0 可信底座 | ✅ 完成（2026-08-17） | A4 网络通路按计划留给 C3 |
| C1 静态 descriptor | ✅ 完成（2026-08-17） | 非空网络规则按计划 fail closed 到 C3 |
| C2 FS ceiling | ✅ 完成（2026-08-17） | sandbox payload 已强制；host Agent launcher 留 C6 |
| C3 网络 broker | ✅ 完成（2026-08-17） | C4 接入结构化网络事件 |
| C4 统一审计 | ✅ 完成（2026-08-17） | fanotify、RET_LOG、SSE 非 C4 阻塞项 |
| C5 动态 phase | ✅ 完成（2026-08-17） | FS ceiling 固定；daemon 多租户认证未实现 |
| C6 Agent 集成/评测 | ⬜ 未开始 | C5 |

建议提交顺序：`fix: make sandbox setup fail closed`、`feat: add static capability
descriptors`、`feat: enforce filesystem capability ceilings`、`feat: route sandbox
egress through policy broker`、`feat: expose bounded capability audit events`、
`feat: enforce phase-scoped capabilities`、`feat: integrate agent tools`。

**下一项唯一推荐工作是 C6 Agent 集成与研究证据。** 不引入行为模型、eBPF 或
预热池；先交付可复现 wrapper、攻击测试和基准脚本。
