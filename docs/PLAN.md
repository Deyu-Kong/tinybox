# tinybox — 问题与修复计划

本文件是 tinybox 代码库的权威逐行审计。它记录 2026-08-16 审查中发现的
每一个缺陷，按严重度分类，并给出具体修法。README 与 AGENTS.md 与此处的
状态保持同步；那些文档里的"完成"仅指满足验收标准**且**无下列开放 P0/P1
项的功能。

> **文档职责。** 本文件管当前代码事实与缺陷；
> [CAPABILITY_PLAN.md](CAPABILITY_PLAN.md) 保留 C0–C6 的已完成实施记录；
> [PRODUCT_PLAN.md](PRODUCT_PLAN.md) 是当前产品化计划；
> [VISION.md](VISION.md) 管产品边界与长期目标。冲突时：当前事实听本文，
> 新工作顺序听 PRODUCT_PLAN，产品边界听 VISION。

> **2026-08-22 产品变更。** 当前目标改为本地、单用户的轻量 Agent 容器系统：
> persistent task + environment model + repeated clean exec + Agent adapters。
> task/exec、environment model、Agent CLI 与分级 adapters 已在 2026-08-22 完成
> M0–M5 验收，已形成实验性的 local Agent container MVP。save/restore/reset
> 仍只是 MVP 后的独立可选评估，不参与
> 项目定义或 MVP 完成门。新的完成门见 PRODUCT_PLAN G0 与 M0–M5。

> **2026-08-22 Agent 接入增强。** OpenCode 与 Pi 现共用 argv-safe task-exec
> runtime，并提供 `agent integrate` 用户级一次安装和 `agent launch` task 生命周期
> 托管。OpenCode 已通过配置加载、真实录制与 launch 验收；Pi 0.73.0 已通过实现、
> 安装、真实模型 Bash tool call 与 task cleanup 验收。Codex 仍仅是整 Agent 非交互 smoke，
> 不声称替换其内置 shell。

约定：
- `file:line` 引用以提交 `b73c7b1`（phase 13）的代码树为准。
- 严重度：**P0** = 隔离/安全被打破 · **P1** = 正确性 bug 或文档声称的功能
  实际不工作 · **P2** = 声称的功能浅薄/不完整 · **P3** = 打磨、技术债、
  非阻塞。
- 每条含：**问题 · 位置 · 影响 · 修法**。

---

## 摘要

> **2026-08-16 M2 后复审：M2 不应视为完全关闭。** 原 M2 提交确实加入了
> 对应代码，但复审发现验收与语义仍有缺口：OCI namespace 子集可能在宿主
> mount namespace 执行初始化；daemon 仍把 child setup/exec 错误记为
> `completed(exit_code=1)`；特殊文件系统的若干关键 mount 错误被忽略；
> `--proxy` 没有代理转接，仅注入环境变量；只读 bind volume 没有可靠 remount。
> 因此 README/AGENTS 的相关能力已降为 ⚠️，在新增回归验收前不得恢复 ✅。

| 原审计分类 | 数量 | 当前状态 |
|----------|-------|--------|
| P0（隔离/安全） | 4 | ✅ 全部解决（M1 完成，2026-08-16） |
| P1（正确性/矛盾） | 5 | 实现曾落地；A1/A2 使相关能力重新开放 |
| P2（功能浅薄） | 5 | P2-1 部分落地但有 A3；其余 4 项开放 |
| P3（打磨） | 6 | 开放（一处附带已修） |

**原审计中的四个 P0 已关闭。** 四个 P0 项在里程碑 M1（2026-08-16）解决：
bridge/veth/NAT 路径被整体删除（Option A——`src/network.rs` 删除，
`--network`/`-p` flags 移除），沙箱现在**始终** unshare `CLONE_NEWNET`
使 `--proxy` 不再共享宿主 netns（隔离 netns + env vars），seccomp 白名单的
逃逸原语被移除且 `clone` 被限制以禁止 `CLONE_NEW*` flags，capability
bounding set 经 `PR_CAPBSET_DROP` 清空。

默认 `tinybox run` 路径具备 namespaces + overlayfs + cgroups + seccomp +
capabilities 的静态骨架，但仍是 **rootful、实验性实现**，不可描述为已验证的
生产安全边界。带显式 OCI namespace 子集、`--dangerous`、无 rootfs 的运行
路径尤其需要单独评估。

### M2 后复审新增的开放项

- **A1（正确性/安全）OCI namespace 子集 ✅ C0 关闭**：`child_main` 无论是否请求
  mount namespace 都执行 mount propagation、rootfs 和 `/proc` 初始化；显式
  `user` namespace 又被静默忽略。必须校验必需 namespace，或只在对应 namespace
  中执行相关初始化，且拒绝不支持的 `user`。
- **A2（正确性）daemon 状态 ✅ C0 关闭**：只有 fork 前错误返回 `Err`；child setup、
  `chdir`、setuid 或 exec 失败均变成 exit code 1，仍标记 `completed`。需要独立
  setup-error pipe/protocol。
- **A3（正确性）特殊文件系统 ✅ C0 关闭**：device bind、devpts、shm、sys mount 多处
  `.ok()`；“完整硬化”没有 fail-closed 保证。`/dev/mqueue` 也未实现。
- **A4（功能）proxy ✅ C3 关闭**：原实现只有隔离 netns + proxy env；没有 loopback bring-up、
  veth、socket relay 或其它宿主代理转接。`127.0.0.1:PORT` 指向沙箱自身，故当前
  代码不能兑现“wget 经宿主 proxy 成功”。C3 现以 helper + broker 关闭该缺口。
- **A5（正确性）只读 volume ✅ C0 关闭**：bind mount 初次调用附加 `MS_RDONLY`，未执行
  `MS_REMOUNT|MS_BIND|MS_RDONLY`；只读语义需要修复并以写失败验收。
- **A6（验证）测试门误导 ✅ C0 关闭**：非 root 下需要特权的集成测试直接返回，cargo 会
  报为通过。报告测试结果时必须区分“编译/非特权测试通过”和“root 验收通过”。

**C0 解决记录（2026-08-17）：** OCI namespace 改为类型化并拒绝未知与
`user`；显式集合必须包含 mount namespace。`CLOEXEC` setup pipe 将
`setup_failed` 与 payload exit 分离。特殊 FS 关键错误改为 fail-closed；volume
移到 pivot 前绑定，拒绝 symlink target，并执行真正的只读 remount。cgroup
验证 v2 controller，CLI 显示实验性警告；CI 将无特权 unit/lint 与 root C0
验收分开。A4 的网络数据通路已由 C3 关闭。

**Capability track（2026-08-17）：** C1 已加入 CLI/API 共用的版本化
`CapabilityDescriptor`、资源 ceiling 与稳定 policy hash；未执行的网络/phase
规则 fail closed。C2 已用 Landlock ABI 强制 sandbox payload 的 FS ceiling，
并以读写、只读、未声明路径和 symlink escape 的 root 验收覆盖。C3 已加入
私有 netns CONNECT helper 与 host broker：仅精确匹配策略中的 host/port，
payload 直接 socket 无路由；本地 fixture、拒绝规则与直连失败均有 root 验收。
C4 已加入每 sandbox 1024 条有界审计环、dropped 计数、事件与 summary API；
runtime、Landlock、cgroup 与 broker allow/deny 决策均产生结构化事件，且不记录
请求 body、token、文件内容或环境变量。C5 已加入经 schema 校验的 phase graph、
generation CAS、合法 next 检查、动态 broker allowlist 与 cgroup limit 更新；FS
ceiling 保持不变，payload 因无宿主路由不能访问控制 API。C6 已加入无 policy
参数的 tool wrapper、host Agent Landlock launcher、三类 workload、攻击验收和
native/tinybox/runc + phase/audit 可复现 benchmark。C0–C6 均已 root 验收；这不
关闭 daemon 认证、rootless、动态更新回滚或研究轨问题。

---

## P0 — 隔离 / 安全 ✅ 已解决（M1）

> 下面四条 P0 均在 2026-08-16 修复（`0531141` 之后的提交）。原文保留为
> 缺陷 + 修法的历史记录。

### P0-1 `--network bridge` 把网络配到了宿主而非沙箱 ✅
- **问题**：设了 `--network` 时，`child_main` **不**把 `CLONE_NEWNET`
  加进 `unshare` flags。父进程随后在子进程仍阻塞在同步 pipe 上时（即
  仍在宿主 netns）调 `network::move_veth_to_ns(child_pid)`。于是 veth 留
  在宿主 netns，`configure_container_network` 把 `172.20.x.y` 分配给一个
  接口并装默认路由**到宿主上**。这是网络泄漏，不是隔离。
- **位置**：`src/sandbox.rs:141-143`（NEWNET 门控为 `proxy.is_none() &&
  network.is_none()`）、`src/sandbox.rs:98`（父调 `move_veth_to_ns`）、
  `src/network.rs:113,122`。
- **影响**：任何 `--network bridge` 沙箱都改动宿主路由/接口。可能破坏
  宿主连通性，是权限边界违例。
- **修法（两选项，需决策——见决策日志 2026-08-16）**：
  - **Option A（与 AGENTS.md 一致）**：删除 `network.rs` 的 bridge/NAT
    路径；保持 proxy-only。`--network` 变 no-op 或移除。恢复文档设计，
    减 ~187 LOC，去掉 `ip`/`iptables` 二进制依赖。
  - **Option B（保留 bridge，修好）**：`child_main` 在 `--network` 时
    始终 unshare `CLONE_NEWNET`；把 `move_veth_to_ns` 调用挪到子进程
    unshare **之后**（重排 pipe 同步，让父进程在子 netns 存在后再移
    veth）。然后 `configure_container_network` 在子进程 unshare 后跑。
    加回归测试断言 `--network bridge` 后宿主路由表不变。
- **建议**：v0.x 选 Option A（契合文档"proxy-based, no bridge"决定与
  "no TUN/TAP, no bridge"约束）。seccomp + cap 稳固后再作为 v1.0 opt-in
  重审 bridge。
- **解决（2026-08-16，取 Option A）**：`src/network.rs` 整体删除；
  `--network` 与 `-p`/`--publish` flags 从 `main.rs` 移除；
  `SandboxConfig.network`/`ports` 字段移除；`sandbox.rs` 的
  bridge/veth/端口映射块移除；`scripts/test_phase11.sh` 删除（它测的是
  已删的 bridge）。`ip`/`iptables` 二进制依赖随之消失。P0-2（下）覆盖
  由此产生的 `--proxy` 语义。

### P0-2 `--proxy` 不提供隔离 ✅
- **问题**：`--proxy <URL>` 只推 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`
  env vars。设 `--proxy` 时不创建 `CLONE_NEWNET`，故沙箱共享宿主 netns；
  任何忽略 proxy env 的二进制都绕过它。
- **位置**：`src/sandbox.rs:141-143`、`src/sandbox.rs:206`
  （`effective_environment`）。
- **影响**：Phase 7 验收"`--proxy ... wget` 成功"是空洞为真——成功是因为
  沙箱有全部宿主网络，而非流量走了代理。反向断言（"`ping 8.8.8.8` →
  unreachable"）仅在**既未** `--proxy` **又未** `--network` 时成立。
- **修法**：始终 unshare `CLONE_NEWNET`。`--proxy` 模式下让 netns 空
  （只有 `lo`）并设 env vars；`--network bridge` 下在子进程内跑 veth
  设置。让 Phase 7 验收断言设了 `--proxy` 时 `ping 8.8.8.8` 失败（只
  loopback 可达）。
- **解决（2026-08-16）**：`child_main` 现在始终插入 `CLONE_NEWNET`
  （`proxy.is_none() && network.is_none()` 门控已移除）。`--proxy` 因此
  产出 loopback-only netns + env vars。回归：`scripts/test_phase7.sh`
  Test 3 断言 `--proxy` 模式无默认路由。

### P0-3 seccomp 白名单含逃逸原语 ✅
- **问题**：白名单允许一些众所周知的容器逃逸/宿主干扰原语：
  - `clone`（无参数过滤）——沙箱进程可
    `clone(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWNS)` 建新 namespace
    绕过缺失的 `unshare`/`setns`/`pivot_root` 拦截。
  - `open_by_handle_at`——配合仍在场的 `CAP_DAC_READ_SEARCH`，是经典
    容器逃逸原语。
  - `process_vm_readv` / `process_vm_writev`——跨进程内存读写。
  - `perf_event_open`、`ioprio_set`、`mbind`、`set_mempolicy`、
    `migrate_pages`、`move_pages`——宿主资源干扰/侧信道。
- **位置**：`src/seccomp.rs:163,349,353,357-358` 及 cap 下调列表
  `src/seccomp.rs:20-38`。
- **影响**：违反 AGENTS.md "default seccomp policy must prevent escape"。
- **修法**：
  1. 给 `clone` 加 `SeccompRule` 参数条件，只允许
     `CLONE_VFORK | SIGCHLD` 风格 flags（或用 `clone3` 限制
     `exit_signal` 替代）。屏蔽 `CLONE_NEW*` bits。
  2. 从白名单移除 `open_by_handle_at`、`process_vm_readv`、
     `process_vm_writev`、`perf_event_open`、`ioprio_set`、`mbind`、
     `set_mempolicy`、`migrate_pages`、`move_pages`。
  3. 把 `CAP_DAC_READ_SEARCH`、`CAP_NET_RAW`、`CAP_SYSLOG`、
     `CAP_AUDIT_*`、`CAP_SETFCAP` 加入下调集。
  4. 加 `// SAFETY:` 注释记录残留风险与 `--dangerous` 逃生口。
- **解决（2026-08-16）**：四条子修都落在 `src/seccomp.rs`。`clone` 现在
  带 `SeccompCmpOp::MaskedEq(0x7E020000)` 规则于 arg0，任何 `CLONE_NEW*`
  bit → SIGSYS；`clone3` 仍不在白名单。九个逃逸/干扰 syscall 已移除。
  `DANGEROUS_CAPS` 8 → 14（加 `CAP_DAC_READ_SEARCH`、`CAP_NET_RAW`、
  `CAP_AUDIT_WRITE`、`CAP_AUDIT_CONTROL`、`CAP_SETFCP`、`CAP_SYSLOG`）。
  规则构建抽出为 `build_rules()` 以便单测。回归：`scripts/test_phase5.sh`
  Test 6（`clone(CLONE_NEWUSER)` → SIGSYS 159）及两个新 seccomp 单测
  （`test_clone_rule_blocks_namespace_flags`、`test_escape_syscalls_excluded`）。
  正常 fork（`clone(SIGCHLD)`）仍过（Test 5）。

### P0-4 capability bounding set 从未清空 ✅
- **问题**：`drop_capabilities` 清 effective/permitted/inheritable/ambient
  集但从不调 `prctl(PR_CAPBSET_DROP, ...)`。沙箱内 exec 的 setuid 二进制
  在 `execve` 时从 bounding set 重新获得已下调的 cap。
- **位置**：`src/seccomp.rs:40-95`（无 `PR_CAPBSET_DROP`）。
- **影响**：exec 了 setuid-root 二进制的沙箱重获 `CAP_SYS_ADMIN` 等，
  cap 下调失效。
- **修法**：清完 effective/permitted/inheritable 后，遍历下调列表对每个
  调 `prctl(PR_CAPBSET_DROP, cap)`。加单测读 `/proc/self/status` 的
  `CapBnd` 并断言危险 cap 不在。
- **解决（2026-08-16）**：`drop_capabilities` 现在遍历 `DANGEROUS_CAPS`
  并对每个调 `libc::prctl(PR_CAPBSET_DROP, cap, ...)`。回归：
  `scripts/test_phase5.sh` Test 7 断言 `CapBnd` 的 `CAP_SYS_ADMIN`（bit 21）
  已清；`tests/phase5.rs::test_capabilities_dropped` 扩展为也断言 `CapBnd`
  的 `CAP_SYS_ADMIN` + `CAP_NET_ADMIN`。

---

## P1 — 正确性 / 矛盾

### P1-1 OCI 支持忽略 `linux.namespaces`（以及几乎其它一切）
- **问题**：`OciConfig` 只反序列化 `process.{args,env}` 与 `root.path`。
  Phase 6 验收 config 含 `linux.namespaces`——tinybox 静默忽略它们，永远
  建同一套 namespace。`root.readonly`、`mounts`、
  `process.{cwd,user,capabilities}`、`linux.{resources,seccomp,sysctl,cgroupsPath}`
  全丢。
- **位置**：`src/oci.rs:7-30`。
- **影响**："Phase 6 ✅ OCI Bundle 支持"有误导——声称"核心 10 字段"
  实际只 honor ~3 个。一个依赖 namespace *子集*（如只 `pid`+`mount`）的
  OCI bundle 会拿到比请求更宽（非更窄）的隔离集。
- **修法**：扩展 `OciConfig` 至少 honor：`root.readonly`（对 overlay 加
  `MS_RDONLY`）、`process.cwd`、`process.user.{uid,gid}`、`hostname`、
  `linux.namespaces`（驱动 `child_main` 里设哪些 `CLONE_NEW*` flag）。把
  其余不支持字段明确记为忽略。更新 Phase 6 验收测试断言一个
  namespace-restricted config 实际限制了 namespace。

### P1-2 `ip`/`iptables` 非零退出被静默当成功 ✅
- **问题**：`network.rs` 全文 `.status().context(...)?` 只传播 spawn 命令
  的 `io::Error`；`ip`/`iptables` 的非零退出被吞掉当成功。
- **位置**：`src/network.rs:40,46,50,67,76,86,97,107,116,126,...`。
- **影响**：在没装 `ip`/`iptables` 的宿主上，或任何规则插入失败时，
  tinybox 报成功但网络是坏的。叠加 P0-1，失败静默且危险。
- **修法**：每个命令包进 helper `fn run(cmd) -> Result<()>`，检查
  `status.success()`，否则 `anyhow::bail!` 带 stderr。
  （若 P0-1 取 Option A 则 moot——`network.rs` 被删。）
- **解决（2026-08-16，M1）**：`network.rs` 整体删除（P0-1 Option A），
  此类 bug 不复存在。`ip`/`iptables` 运行时依赖随之消失。

### P1-3 daemon 把失败与完成的沙箱混为一谈
- **问题**：`create` 在成功与出错时都设 `status="completed"` 且
  `exit_code = result.ok()`；失败时 `exit_code=None` 但仍计为 "completed"。
  `metrics` 算 `completed = total - running`，故出错沙箱抬高完成计数。
- **位置**：`src/daemon.rs:105`（`exit_code = result.ok()`）、`:143`。
- **影响**：`/metrics` 与 `GET /api/sandboxes` 误报健康；运维者无法区分
  崩溃沙箱与成功沙箱。
- **修法**：引入 `status` 值 `{running, completed, failed}`。出错时设
  `status="failed"`，捕获 `exit_code` 与 `error` 串。暴露
  `tinybox_sandboxes_failed` 为独立 Prometheus 计数器。

### P1-4 daemon `CreateRequest` 不能设多数沙箱选项
- **问题**：HTTP `CreateRequest` 只接受 `rootfs`、`command`、
  `memory_limit_mb`、`proxy`。它构建的 `SandboxConfig` 把 cpus、pids_limit、
  volumes、ports、network、hostname、env、image、oci、dangerous 全硬编码。
- **位置**：`src/daemon.rs:59-95`。
- **影响**：API 无法用 CLI 暴露的功能；Phase 8 验收只验了最简的
  `sleep 30`。
- **修法**：扩展 `CreateRequest` 加可选 `cpus`、`pids_limit`、`volumes`、
  `ports`、`network`、`hostname`、`env`、`image`、`oci`。拒绝 API 传
  `dangerous=true`（或要求显式 opt-in flag）以避免远程禁沙箱 footgun。

### P1-5 `exec.rs` 是 23 行 `nsenter` 包装，有缺口
- **问题**：`exec_in_container` shell 调 `nsenter -t <pid> -m -u -n -p`，
  缺 `-i`（IPC）、`-U`（user）、`-C`（cgroup）。无 TTY 分配，无
  `--cwd`/`--env`/`--user`，不校验 `<pid>` 是 tinybox 沙箱。
- **位置**：`src/exec.rs:4-16`。
- **影响**：exec 的进程不共享 IPC/user/cgroup namespace；交互 shell 无
  控制终端；任意宿主 PID 都可被定向（若 `exec` 接入 daemon API 则是提权
  footgun）。
- **修法**：用 `nix::sched::setns` 替换 `nsenter` shell 调用，对目标各
  namespace（读 `/proc/<pid>/ns/*` 符号链接）逐一 setns。加 `-i/-U/-C`
  等价物。在 `daemon::AppState` 跟踪沙箱 PID，拒绝不在集合内的 PID。加
  `--cwd`/`--env`/`--user` flags。

---

## P2 — 功能浅薄 / 不完整

### P2-1 rootfs 缺 `/dev`、`/tmp`、`sysfs`；只 mount 了 `/proc`
- **位置**：`src/sandbox.rs:217`（`mount_proc`）、`src/rootfs.rs`。
- **修法**：pivot 后，在 `/dev` mount `tmpfs`，建 `/dev/pts`、`/dev/shm`、
  `/dev/mqueue`；从宿主 bind `/dev/null`、`/dev/zero`、`/dev/urandom`、
  `/dev/tty`；在 `/tmp` mount `tmpfs`（带容量上限）；在 `/sys` mount
  `sysfs`（只读）。honor OCI `root.readonly` 给 overlay 加 `MS_RDONLY`。

### P2-2 cgroup：无 v2 校验、无控制器启用、swap 硬编码
- **位置**：`src/cgroup.rs:23,35,38-51`。
- **修法**：校验 `/sys/fs/cgroup/cgroup.controllers` 存在（v2）。必要时
  向父 `cgroup.subtree_control` 写 `+memory +cpu +pids`。让 `swap.max`
  可配（默认 0）。加 `io.max` 与 `cpu.weight`。

### P2-3 镜像存储：无内容寻址、无分层、无元数据
- **位置**：`src/image.rs`。
- **修法**：镜像存为 `<store>/<sha256>/`，别名符号链接
  `<store>/aliases/<name> -> ../../<sha256>`。支持分层解包（whiteouts）。
  每镜像写 `metadata.json`（created、size、parent、labels）。

### P2-4 registry 拉取：blob 全在内存、无 config、无 digest 校验、仅 Docker Hub
- **位置**：`src/registry.rs:85-100,103,112`。
- **修法**：blob 流式写临时文件（避免 OOM）。拉取 config blob，在用户省略
  `command` 时把 `Cmd`/`Entrypoint`/`Env`/`WorkingDir` 作默认命令。校验
  `docker-content-digest` 与 manifest 一致。支持
  `registry-host[:port]/repo:tag` 解析（在第一个 `/` 处切）。加 HTTP 超时
  与重试。

### P2-5 daemon：无持久化、无鉴权、无流式/日志端点
- **位置**：`src/daemon.rs`。
- **修法**：把 `AppState` 持久化到 `$TINYBOX_STATE_DIR/sandboxes.json`。加
  `GET /api/sandboxes/:id/logs`（流式 stdout/stderr，捕获到文件）。加
  bearer-token 鉴权（`--auth-token` flag）。P1-5 修好后加 `POST
  /api/sandboxes/:id/exec`。加 SIGTERM 优雅关闭。

---

## P3 — 打磨 / 技术债

### P3-1 `tracing` 已批准但未用；日志是 `eprintln!`
- **修法**：用 `tracing::{info,warn,error}` 替换 `eprintln!`，在 `main`
  加 `tracing_subscriber::fmt` 初始化。verbose 输出由 `-v` 门控。

### P3-2 `signal_to_int` 对未映射信号返回 0
- **位置**：`src/sandbox.rs:266-284`。
- **修法**：对任何未映射信号默认 `128 + signum`；留哨兵值（`255`）表
  "被未知信号杀死"。

### P3-3 `parse_port_spec` 只支持 `host:container` TCP
- **位置**：`src/sandbox.rs:288`。
- **修法**：支持 `ip:host:container`、`port-range`、`udp` 后缀
  （`8080:80/udp`）。P0-1 Option A 下 moot。

### P3-4 `mount_proc` 用 `.ok()` 吞掉 `create_dir_all` 错误
- **位置**：`src/sandbox.rs:217`。
- **修法**：传播错误；仅当 `/proc` 已 mount 时才对 mount `.ok()`。

### P3-5 双重 cgroup 清理（`Drop` + 手动 `drop(cg)`）
- **位置**：`src/cgroup.rs:73-80`、`src/sandbox.rs:117`。
- **修法**：移除手动 `drop`；靠 `Drop`。

### P3-6 测试卫生：共享 `TINYBOX_IMAGE_DIR` env 跨测试被改
- **位置**：`src/image.rs` 测试。
- **修法**：每测用 `tempfile::TempDir`，env 只在进程内设。

---

## 修复路线图

排序使每个里程碑后代码树处于可辩护状态。提交沿用现有 `phase N:` / `fix:`
约定；每个里程碑打 tag。

### 里程碑 M0——"诚实基线"（无行为变更）✅
1. 加本 `PLAN.md`。✅（提交 `0531141`）
2. 修正 README 徽章/状态与 AGENTS 决策日志反映真实状态。✅（提交
   `0531141`）
3. README 已加实验性警告；`--help` 尚无同等警告。⚠️

### 里程碑 M1——关闭 P0 隔离漏洞 ✅（2026-08-16）
1. ✅ P0-1 Option A：`src/network.rs` 删除；`--network`/`-p` flags 移除；
   `ip`/`iptables` 运行时依赖消失。
2. ✅ P0-2：`child_main` 始终 unshare `CLONE_NEWNET`；`--proxy` =
   loopback-only + env vars。
3. ✅ P0-3：`clone` 经 `MaskedEq(0x7E020000)` 限制（禁 `CLONE_NEW*`）；
   九个逃逸/干扰 syscall 移除；`DANGEROUS_CAPS` 8 → 14。
4. ✅ P0-4：`drop_capabilities` 经 `PR_CAPBSET_DROP` 清 bounding set。
5. ✅ 回归测试：`test_phase5.sh` Tests 5–7（正常 fork ok、
   `clone(CLONE_NEWUSER)` → SIGSYS、`CapBnd` 已清）；`test_phase7.sh`
   Test 3（`--proxy` 无默认路由）；seccomp clone 规则 + 排除 syscall 的
   单测。所有验收门绿。

### 里程碑 M2——让声称的功能真正能用 ⚠️ 复审后重开（2026-08-16）
1. ✅ P1-1：honor OCI `linux.namespaces`、`root.readonly`、`process.cwd`、
   `process.user`。
2. ✅ P1-3：daemon 状态 `{running,completed,failed}` + 失败计数器。
3. ✅ P1-4：扩展 `CreateRequest`；拒绝远程 `dangerous`。
4. ✅ P1-5：`exec` 走 `setns`，namespace 完整、PID 校验、带 TTY。
5. ✅ **P2-1 已提前**（原在 M3）：完整 `/dev`、`/tmp`、`/sys` 设置。当时
   R0 验收需要真跑 `pip install`，而它需要 `/dev/null`、可写 `/tmp` 等。
6. ⚠️ 当时记录为 `cargo test`（59 测试）+ clippy 及若干 acceptance 全绿；
   复审发现测试未覆盖 A1–A5，且非 root 的集成测试会静默跳过。

> **历史说明：** 原 R0–R6 研究路线已被 2026-08-20 的
> [PRODUCT_PLAN.md](PRODUCT_PLAN.md) 取代；本段仅解释 M2 当时的排序。

### 里程碑 M3——纵深
1. P2-2：cgroup v2 校验 + 控制器启用。
2. P2-3/P2-4：内容寻址镜像 + registry config blob 拉取 + 流式。
3. P2-5：daemon 持久化 + 日志 + 鉴权 + exec 端点。

### 里程碑 M4——打磨
1. P3-1 至 P3-6。

### 延伸（修复轨 v1.0 明确不做）
- rootless 运行（`CLONE_NEWUSER` + uid 映射，研究轨 R5）
- cgroup namespace
- UDP 端口映射 / hairpin NAT（仅在保留 bridge 时——bridge 已在 M1 删，
  故此项休眠）
- 多机任何东西

> 新的 Agent Tool Sandbox 产品化工作在 [PRODUCT_PLAN.md](PRODUCT_PLAN.md)，
> 不在本缺陷审计中追踪。

---

## 验收门（打 tag 前必须过）

- `cargo test && cargo clippy -- -D warnings`（既有门）。
- 新：`scripts/test_phase7.sh` 断言 `--proxy` 跑后宿主路由表不变。
- 新：`scripts/test_phase5.sh` 断言沙箱内 `clone(CLONE_NEWUSER)` 返回
  `EPERM`（实为 SIGSYS 159）。
- 新：`scripts/test_phase6.sh` 断言一个只请求 `{pid,mount}` namespace 的
  OCI config 实际限制到那些。
- 新：`scripts/test_phase8.sh` 断言 `/metrics` 在故意崩掉的沙箱后报
  `tinybox_sandboxes_failed`。

---

## 状态图例（供 README/AGENTS 用）

更新各 phase 状态时，用：
- ✅ **works**——过验收、无开放 P0/P1 项。
- ⚠️ **partial**——能跑但有开放 P1/P2 项（见 PLAN.md）。
- ❌ **broken**——有开放 P0 项或不过验收。

当前各 phase 状态（M2 后，2026-08-16）：

| Phase | Feature | Status | Open items |
|-------|---------|--------|------------|
| 1 | skeleton + CLI + exec | ✅ | — |
| 2 | namespaces (pid/mount/uts/net) | ✅ | OCI 子集 fail-closed（A1/C0） |
| 3 | overlayfs + pivot_root | ✅ | 特殊 FS fail-closed（A3/C0） |
| 4 | cgroup limits | ⚠️ | P2-2（无 v2 校验，swap 硬编码） |
| 5 | seccomp + caps | ✅ | —（P0-3、P0-4 已在 M1 修） |
| 6 | OCI bundle | ⚠️ | namespace 已 fail-closed；仍仅支持字段子集 |
| 7 | network（isolated + policy broker） | ✅ | C3 本地 fixture、拒绝与直连失败验收 |
| 8 | daemon API | ⚠️ | setup_failed 已区分；仍有 P2-5 持久化/鉴权/日志 |
| 9 | local images | ⚠️ | P2-3 |
| 10 | registry pull | ⚠️ | P2-4 |
| 11 | ~~network bridge~~ | 🗑 移除 | M1 移除（Option A）；原 P0-1 |
| 12 | volumes | ✅ | pivot 前 bind、symlink 防护、只读 remount（A5/C0） |
| 13 | exec | ⚠️ | setns + 基础 PID 校验已实现；未分配 PTY，验证覆盖有限 |
