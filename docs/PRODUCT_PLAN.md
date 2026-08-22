# tinybox MVP 实施计划

> **ACTIVE，2026-08-22。** 本计划服务于“本地 Agent 轻量容器系统”的定位。
> 当前代码事实与缺陷听 [PLAN.md](PLAN.md)，产品边界听
> [VISION.md](VISION.md)。未通过对应完成门的功能不得在 README 标为完成。

## 1. MVP 目标

交付一条完整而小型的本地 Agent 容器链路，而不是一个目录快照工具：

```text
tinybox agent run <agent> <workspace>
                 │
                 ▼
          persistent task
          ├── environment model
          ├── repeated clean exec
          ├── isolation and cleanup
          └── Agent adapters
```

MVP 要证明：

1. 用户不写项目 Dockerfile，也能让常见 CLI Agent 使用一个本地容器环境；
2. 一个 Agent session 可以持续使用依赖、home 和 cache，但每次 tool process 都能
   可靠回收；
3. OpenCode 至少有一个不修改上游核心的可用接入；
4. Codex、Pi Agent 和通用 CLI Agent 有诚实的支持矩阵；
5. “轻量、方便”由安装步骤、延迟、RSS 和磁盘数据支持。

### 已确认的产品决策

| 问题 | 决策 |
|---|---|
| tinybox 是什么 | 面向本地 Coding Agent 的轻量 Linux 容器系统，不是权限管理产品 |
| 为何不直接用 Docker | 缩小到 Agent 所需的 task、exec、environment、cleanup 与 adapter，减少镜像和容器编排负担 |
| task 生命周期 | 一个 Agent session 一个长期 task，而不是每次 tool call 创建新容器 |
| tool 生命周期 | 每次 tool call 在 task 内启动独立、可超时并可完整回收的 exec |
| 复杂宿主环境 | 通过 host/rootfs/profile 三种显式环境来源复用；不自动复制整个宿主系统 |
| 源码状态 | direct workspace 交给宿主文件系统与 Git；tinybox 不替代 Git |
| save/restore | MVP 后的环境生命周期实验，不是核心能力 |
| 安全边界 | 限制同一用户 Agent 命令的事故影响；不防成功的宿主内核利用，不作为敌对租户边界 |
| 首个集成 | OpenCode 工具级 adapter；Codex 先做整 Agent 包装验证；Pi 先验证扩展接口 |

## 2. 产品能力与范围

### MVP 包含

- 本地单机、单用户、rootful Linux；
- `agent run/list/stop/destroy` 用户入口；
- task create/get/exec/destroy 底层生命周期；
- workspace、private home、private cache、rootfs writable state 和显式 volumes；
- host、rootfs、profile 三种环境来源的最小可用子集；
- command、cwd、env、stdin/stdout/stderr、exit code、timeout 和进程树回收；
- OpenCode adapter、通用 CLI wrapper；
- Codex 整 Agent 包装与 Pi Agent adapter 的可行性结论；
- bare/tinybox 与 cold/warm 对照演示。

### MVP 不包含

- CRIU、进程内存、文件描述符或 TCP 连接恢复；
- Git 替代、源码 merge 或 workspace 版本历史；
- 自动分支搜索、并行 rollout 或状态树；
- Dockerfile build、Compose、Kubernetes、完整 OCI 和 registry 平台；
- rootless、多用户、多租户、集群、GPU、Windows/macOS；
- 新的行为推断、自动审批或动态权限研究。

## 3. 核心数据模型

### 3.1 Agent task

- 一个 Agent session 对应一个 task；
- task 持有 namespace、cgroup、mount view、workspace 和 environment；
- task ID/token 由宿主 adapter 持有；
- daemon 或 task 异常退出后必须回收 keeper、子进程、mount、cgroup 和状态目录。

### 3.2 Environment

Environment 不是固定的单个目录，而是一组生命周期一致的状态：

```text
Task Environment
├── immutable base/rootfs
├── rootfs writable layer
├── private home
├── private package/build caches
├── declared task volumes
├── selected read-only host tools
└── sanitized environment variables
```

### 3.3 Workspace

- direct 模式将宿主 workspace 映射进 task，修改立即回到宿主；
- managed workspace 可留作后续选项，但 MVP 不重新实现 Git；
- 非 Git 项目仍可运行，但源码恢复由用户或上层工具负责。

### 3.4 Exec

- 每次 exec 是 task 内的新进程树；
- 不继承上一条命令的 shell cwd、局部变量和偶然后台进程；
- 继承 task workspace 和 environment；
- timeout、取消和正常退出后都回收整个 exec cgroup；
- 长期服务若以后支持，必须显式建模，不能伪装成普通后台命令。

### 3.5 可选的环境生命周期增强

save/restore 可以增强 Agent 的反复试错体验，但不是 tinybox 成立的必要条件，也不
进入核心 MVP 完成门。只有在 task、environment、exec 和 Agent adapter 可用后，
才做独立 feasibility spike，回答：

- 应保存 rootfs writable layer、home、cache 和哪些 volumes；
- 目录复制、OverlayFS layer rotation 或已有文件系统 snapshot 哪个足够；
- 它相对 Git、重建环境和 Docker volume/image workflow 是否真的减少操作与时间；
- 是否值得进入后续版本。

若实现，仍只做 cold environment checkpoint，不做 CRIU、进程内存或连接恢复。

## 4. 环境来源

### Host 模式

```bash
tinybox agent run --env host opencode .
```

宿主 `/` 作为基础 rootfs，工具链和系统库只读复用；home、cache 和其他写入进入
task 私有状态。必须避免把宿主整个 home、凭据和 daemon socket 暴露进 task。

### Rootfs 模式

```bash
tinybox agent run --root /path/to/rootfs opencode .
```

使用用户提供的 rootfs 和私有 writable layer。它适合需要更独立环境的项目，但
仍不要求 tinybox 实现完整镜像构建平台。

### Profile 模式

```bash
tinybox agent run --profile rust opencode .
```

profile 是小型、受信的 environment manifest。MVP 只需要 `host-basic`、`rust`、
`node`、`python` 等少数手工 profile，不做自动工具链闭包推导。

## 5. 实施里程碑

### G0：设计冻结（进入编码前） ✅ 2026-08-22

- 固定 task、environment、exec、adapter 四个术语及其所有权；
- 固定长期 task + 短期 exec 的进程与资源生命周期；
- 固定 host/rootfs/profile 的可见路径、可写路径和凭据边界；
- 固定 OpenCode adapter 的输入输出与 fail-closed 行为；
- 将当前工作树中的 task 原型仅作为待审实现，不以既有代码反推设计；
- 为 M0–M3 各写出可执行验收矩阵，再开始功能实现。

完成门：架构、API、威胁边界、验收矩阵无互相矛盾项；README 不把目标接口写成
已实现能力。

完成记录：`c321f2f` 固定了定位、生命周期、环境来源、集成边界及 M0–M5
验收门。随后才开始审计工作树中的 task 原型。

### M0：收稳 persistent task ✅ 2026-08-22

- 修复 daemon 退出后的 keeper/cgroup/mount 残留；
- 通用 sandbox DELETE 不能绕过 task token；
- 加强 keeper PID 身份与 cgroup 绑定；
- destroy 等待或验证资源清理完成；
- 完成 timeout/cancel/环境变量/cwd 的契约；
- root 验收 task create、两次 exec、timeout、destroy 和 daemon crash。

完成门：每条路径都无 PID、exec cgroup、task cgroup、mount 和状态目录残留。

完成记录：task API 使用 secret token；keeper 以 PID start time 与 cgroup 双重校验；
每次 exec 使用独立 cgroup，正常退出、后台进程和 timeout 均执行整组回收；destroy
等待 task cgroup 为空并删除后才返回。旧 sandbox DELETE 对 task 返回 403。daemon
遭 `SIGKILL` 时 PDEATHSIG 终止 supervisor/keeper，替代 daemon 仅清扫已空的孤儿
task cgroup，不触碰其它活动 daemon。MVP 的 cancel 契约为 timeout 或 destroy 整个
task，尚不提供保留 task 的独立 exec cancel API。root 验收见
`tests/task_session.rs`。

### M1：Environment model ✅ 2026-08-22

- 引入 environment manifest 与 task state directory；
- 拆分 base rootfs、writable state、private home、cache 和 volumes；
- 落地 host 与 rootfs 两种来源；
- 实现 `host-basic`，再为 Rust/Node/Python 建最小 profile；
- 所有 host tool/cache 映射明确标注 read-only、private-write 或 direct；
- 用三个小 fixture 验证依赖在连续 exec 中持久化。

完成门：三个生态至少各有一个 build/test smoke；宿主凭据和非声明 home 不可见；
task 内环境写入不污染对应宿主目录。

完成记录：task 创建接受版本化 environment manifest 的 host、rootfs 或 profile
来源，并在 `/var/lib/tinybox/tasks/<id>` 保存 overlay writable layer、private home、
cache 与映射清单。workspace 为 direct，home/cache 为 private-write，发现的用户态
rustup/nvm 工具链只获得 read+execute；未知 profile 与非法 volume fail closed 且不
残留 state。Rust、Node、Python 均通过真实工具 smoke 和跨 exec cache 持久化 root
验收；合成 secret、symlink escape 与只读工具路径写入均被拒。为兼容现代 rustc/node，
seccomp 通过前置 filter 将不可安全参数过滤的 `clone3`/`io_uring_*` 返回 ENOSYS，
使其回退到受参数过滤的 `clone`/epoll，而不是直接放行这些 syscall。

### M2：Agent CLI 与生命周期 ✅ 2026-08-22

实现用户入口：

```text
tinybox agent run
tinybox agent list
tinybox agent stop
tinybox agent destroy
```

- CLI 自动创建 task、准备 environment、启动 adapter/Agent 并清理；
- 普通用户不需要直接拼 HTTP JSON；
- 明确 foreground、detach 和 attach 的 MVP 支持程度；
- root helper 与 Agent 进程的职责和凭据分离；
- 错误不得静默回退到宿主裸执行。

完成门：一条命令启动通用 CLI workload，一条命令销毁且无残留。

完成记录：`tinybox agent run/list/stop/destroy` 已落地，并额外提供 adapter 可用的
`agent exec`。foreground 同步返回 stdout/stderr/exit code并自动 destroy；detach
只创建长期 task，不在后台偷偷启动命令；MVP 不支持 attach/TTY streaming。token
仅保存在宿主 `/run/tinybox/agents` 的 0600 record，sandbox 看不到；daemon 连接或
exec 失败明确报错且绝不回退宿主。stop 回收 PID/mount/cgroup 但保留 environment
state，destroy 再删除 state 与 record。task ID 包含 daemon PID 与序号，避免多个
本地 daemon 共享 cgroup/session 名。root 验收见 `tests/agent_cli.rs`。

### M3：Agent integrations

OpenCode：

- custom tool 将 bash/tool run 映射为 task exec；
- adapter 不接受 Agent 提供 policy、mount source 或任意 host path；
- command、cwd、timeout、输出截断和非零退出码与上游契约一致。

Codex：

- 优先验证 `tinybox agent run -- codex` 整 Agent 包装；
- 验证 TTY、配置、凭据、session resume 与嵌套 sandbox；
- 不声称透明替换 Codex 内置 shell。

Pi Agent：

- 阅读官方扩展接口并做 adapter spike；
- smoke test 通过后才进入支持列表。

完成门：每个 Agent 标记为 `supported`、`experimental` 或 `unsupported`，并附可复现
命令；至少 OpenCode 的确定性 adapter contract tests 全绿。

### M4：Demo 与度量

演示 A：bare 与 tinybox 都完成正常 build/test；tinybox 额外证明 timeout、后台进程
和 task destroy 得到完整回收。

测量：

- 安装和启动步骤数；
- bare、cold task、warm exec p50/p95；
- daemon/task 空闲 RSS；
- environment 初始化与连续 exec 的磁盘增量；
- destroy 后残留检查。

完成门：脚本输出机器可读结果和人类摘要；README 的性能表述只引用实测数据。

### M5：本地安装与文档

- 一条构建/安装命令和一条 Agent 启动命令；
- 自动内核/cgroup/Landlock 检测；
- 临时 daemon 或 systemd 用户体验；
- 支持矩阵、清理、卸载和故障排查；
- rootful、Linux-only 和同内核安全边界保持醒目。

完成门：在干净的受支持 Linux 上按 README 可完成安装、root 验收和 demo。

### M6：可选增强评估

在核心 MVP 完成后分别评估：

- environment save/restore/reset；
- managed workspace；
- environment profile 自动发现；
- Codex/Pi 的更深工具级集成；
- rootless launcher。

每项都要先写对照用例和成本数据，再决定是否实现。save/restore 的对照必须包括
Git、重建环境以及等价 Docker workflow；不能因为已经讨论过就自动成为主功能。

## 6. 核心 API 方向

```text
POST   /api/tasks
GET    /api/tasks/:id
POST   /api/tasks/:id/exec
DELETE /api/tasks/:id
```

API 只服务宿主侧 adapter。task token、policy、mount source 和环境选择均由可信
宿主端持有；Agent/tool payload 只能提交 command、task 内 cwd、stdin 和 timeout。
首版不承诺 daemon 重启后恢复 task，也不开放远程监听。

## 7. 测试门

每个里程碑至少运行：

```bash
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

task/environment 必须另跑 root 集成测试。非 root 提前返回只能报告 skip。
验收使用 `/var/tmp` 合成 fixture，不读取真实用户凭据。

## 8. 停止条件

- 产品路线重新围绕 checkpoint 展开：停止，回到本地 Agent sandbox 核心；
- host 模式实际把整个 home 或可写工具链暴露给 task：不得演示为安全复用；
- daemon crash 仍残留 keeper/mount：M0 未完成，不进入 Agent 集成；
- OpenCode 可静默绕过 adapter 执行裸 shell：只能称实验性工具级接入；
- rootful 安装抵消“方便”：优先改本地 UX，不继续增加高级功能；
- 没有数据支持“更快”：只能使用“功能子集、链路更短”的轻量表述。

## 9. 版本完成定义

G0 与 M0–M5 全部关闭后，README 才能将状态改为“local Agent container MVP”。M6 是可选
增强，不阻塞 MVP。在此之前，task、environment 和 Agent adapter 必须逐项标记
实际验收状态。
