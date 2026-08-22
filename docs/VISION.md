# tinybox — 本地 Agent 轻量容器系统

> **产品北极星，2026-08-22。** 当前代码事实与缺陷听
> [PLAN.md](PLAN.md)，实施顺序听 [PRODUCT_PLAN.md](PRODUCT_PLAN.md)，
> C0–C6 的历史能力实现见 [CAPABILITY_PLAN.md](CAPABILITY_PLAN.md)。

## 1. 一句话定位

> tinybox 是一个面向本地 Coding Agent 的轻量级 Linux 容器系统：借鉴
> Docker 的 namespace、cgroup 与分层文件系统，以及 E2B 的持久 sandbox、
> 重复 exec 和环境管理体验，为 OpenCode、Pi Agent、Codex 等 CLI Agent
> 提供小型、自托管、无需完整容器平台的执行环境。

这里的“容器系统”指 task 生命周期、隔离文件系统、资源回收和重复 exec；权限询问、
组织 IAM 与 Agent 决策仍由上层负责。tinybox 可以执行静态安全约束，但不以“权限
管理系统”作为产品定位。

tinybox 不以功能数量与 Docker 竞争，也不以云端隔离强度与 E2B 竞争。它把个人
开发者和单租户 runner 真正需要的容器能力裁剪成一条短路径：

```text
Agent session → persistent tinybox task → repeated tool exec → destroy
                         │
                         ├── workspace
                         ├── environment model
                         ├── repeated clean exec
                         └── isolation + lifecycle
```

## 2. 为什么存在

本地 Coding Agent 有两个常见选择：

1. 直接在宿主 shell 执行，接入简单，但命令、构建脚本和第三方依赖共享宿主的
   进程、网络、文件系统与资源；
2. 使用完整 Docker、远程 sandbox 或 microVM，隔离和生态更完整，但个人用户
   往往还要准备镜像、同步环境、管理容器，或把代码送到远端。

tinybox 服务两者之间的窄场景：

- 代码和 Agent 都在用户自己的 Linux 机器或专用 VM；
- 用户需要本地、低配置成本的隔离执行；
- 一个 Agent session 需要连续运行多条命令并保留开发环境；
- 用户不需要 Compose、Kubernetes、完整 registry 或多租户控制面；
- 失败尝试产生的环境变化应该能够丢弃或恢复。

项目成立的理由不是“Docker/E2B 做不到”，而是：

> 个人用户不一定需要通用容器和托管 sandbox 的全部能力；tinybox 将本地 Agent
> 所需的子集做成更直接的默认体验。

## 3. “轻量”的可验证含义

“轻量”不等于未经测量就声称比 Docker 启动更快。tinybox 使用以下可检查定义：

- 单个 Rust 项目和小型本地 daemon；
- Linux-only，不包含 guest OS 或 VMM；
- 不依赖 Docker daemon、containerd 或 runc；
- host-rootfs 模式无需为每个项目制作镜像；
- 不实现完整 OCI、镜像构建、Compose、集群调度和多租户平台；
- API 只围绕 Agent 所需的 agent、task、environment、exec 和 destroy；
- 安装步骤、cold/warm 延迟、RSS 和磁盘增量必须由演示脚本实测。

在基准完成前，文档只能声称产品范围更小、链路更短，不能声称性能必然更优。

## 4. 核心能力

### 4.1 Agent task

一个 Agent session 对应一个长期 task。task 持有 namespace、cgroup、mount 视图、
workspace 与私有环境；task ID 和 secret token 由宿主 adapter 持有。

### 4.2 独立 tool exec

每次 shell/tool run 是 task 内的新进程树，继承 task 文件环境但不依赖上一次
shell 的 cwd、变量或后台进程。完成、取消或超时后回收整个 exec cgroup。

### 4.3 开发环境模型

task 环境不是固定的 `/environment` 目录，而是 base rootfs、rootfs writable
layer、private home、private cache、显式 volumes、只读宿主工具和净化环境变量的
组合。MVP 支持 host、rootfs 和少量手工 profile，目标是在不编写项目 Dockerfile
的情况下运行常见 Rust、Node 和 Python 项目。

### 4.4 可选的环境生命周期增强

save/restore/reset 可能帮助 Agent 回滚依赖和环境试错，但不定义 tinybox，也不阻塞
核心 MVP。只有 task、environment、exec 和 adapter 可用后，才比较 Git、重建环境、
Docker workflow 和 cold environment checkpoint 的真实成本，再决定是否实现。

```text
Git                         tinybox
  └── source history          └── local Agent execution environment
```

若未来实现，只考虑冷环境恢复，不做 CRIU、进程内存或完整 VM pause/resume。

### 4.5 容器隔离与回收

namespaces、cgroup v2、seccomp、capability drop、Landlock 和私有网络共同限制一次
Agent 命令的事故影响。它们是容器执行底座，不取代 Agent 自己的 allow/ask/deny
或用户审批。

## 5. Agent 接入形态

不同 Agent 不强行使用同一种适配方式：

| Agent 类型 | 首选接入 | MVP 承诺 |
|---|---|---|
| OpenCode | custom tool/插件将 shell 路由至 task exec | 首个工具级集成与演示目标 |
| Pi Agent | 在验证其扩展接口后提供 adapter | 未验证前只列计划，不声称支持 |
| Codex CLI | `tinybox agent run -- codex` 外层包装 | 先验证整 Agent 容器模式；不声称透明替换内置 shell |
| 其他 CLI Agent | 通用 wrapper | 只保证进程包装，不保证专用 UI 集成 |

工具级模式让 Agent/LLM 凭据留在宿主；整 Agent 模式提供更完整的执行环境包裹，
但要单独处理配置和凭据注入。README 必须区分“设计目标”“实验适配”和“已验收”。

## 6. 与 Docker、E2B 的关系

```text
Docker：通用本地容器平台和完整生态
E2B：   托管 Agent sandbox 平台
tinybox：本地、单用户、Agent 专用的容器能力子集
```

tinybox 借鉴：

- Docker/runc 的 Linux 隔离、COW rootfs 和资源生命周期；
- E2B 等 Agent sandbox 的持久 session、重复 exec、环境持久化和 SDK 形态。

tinybox 明确不宣称：

- 比 microVM 有更强的内核逃逸边界；
- 比 Docker 拥有更完整的容器能力；
- 当前目录级 checkpoint 是新的 CR 技术；
- rootful 实验版本可作为公有云跨租户最终安全边界。

详细对比见 [COMPETITIVE_LANDSCAPE.md](COMPETITIVE_LANDSCAPE.md)。

## 7. 首个完整使用场景

用户在本地复杂项目中运行 OpenCode：

1. adapter 创建一个 task 并挂载当前 workspace；
2. OpenCode 的 shell 调用通过 `task exec` 执行；
3. 选择 host/rootfs/profile 环境，第一次安装依赖后在后续命令中继续存在；
4. 超时命令、后台子进程和 task 退出得到完整回收；
5. 成功结果仍以普通文件和 Git commit 留在宿主项目中。

用户体验目标：

```bash
tinybox agent run opencode .
```

这条命令目前是目标接口，不是已完成事实。

## 8. 边界与非目标

- 只支持 Linux；当前 rootful、实验性；
- 同一宿主内核上的恶意租户不是目标；高风险部署需要外层 VM/microVM；
- 不处理成功的宿主内核利用；
- 不实现完整 OCI、Dockerfile build、Compose、Kubernetes 或多节点；
- 不做 GPU、Windows/macOS、用户态网络和 bridge；
- 不保存进程内存、活跃连接或完整 Agent 会话；
- 不替代 Git；workspace 历史、diff 和 merge 仍由 Git 完成；
- 不把行为推断、自动审批和组织 IAM 放入 runtime 核心。

## 9. 成功标准

项目是否成立由一个可复现的本地演示判断，而不是功能数量：

- 同一 OpenCode 任务在 bare 与 tinybox 模式下都能完成正常构建和测试；
- tinybox 模式不要求项目专用 Dockerfile；
- Rust、Node、Python smoke workload 能复用 task home/cache/environment；
- 多次 exec 复用 task 环境，但不残留普通后台进程；
- task destroy 后没有残留 PID、cgroup、mount 或私有状态目录；
- 报告 cold/warm 延迟、RSS 与磁盘增量；
- 所有特权验收明确显示 pass/skip/fail，不能把非 root skip 当成功。

## 10. 文档职责

| 文档 | 职责 |
|---|---|
| [../README.md](../README.md) | 用户入口、已实现状态和快速开始 |
| [PLAN.md](PLAN.md) | 当前代码事实与开放缺陷 |
| [PRODUCT_PLAN.md](PRODUCT_PLAN.md) | MVP 实施顺序和完成门 |
| [OPENCODE_DEMO.md](OPENCODE_DEMO.md) | OpenCode 对照演示规范 |
| [COMPETITIVE_LANDSCAPE.md](COMPETITIVE_LANDSCAPE.md) | 相邻产品与诚实对比 |
| [CAPABILITY_PLAN.md](CAPABILITY_PLAN.md) | C0–C6 历史能力轨记录 |
