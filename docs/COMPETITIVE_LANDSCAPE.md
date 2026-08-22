# 本地 Agent 容器相邻产品调研

> 调研日期：2026-08-20，定位复审：2026-08-22。只依据各项目官方文档描述产品能力；“可学习点”是
> tinybox 的设计推论，不代表对相应产品做过独立安全审计。

## 1. 结论

市场已经证明“给 Agent 一个可编程沙箱”是明确需求。tinybox 不与云端
microVM 平台竞争跨租户隔离，也不与 Docker 竞争通用容器生态。产品定位是：

- 本地、自托管、单用户；
- Docker 风格 Linux 隔离的精简子集；
- E2B 风格 persistent task、repeated exec 和环境生命周期；
- 无项目 Dockerfile 的目标使用路径；
- Git 管源码、tinybox 管 task 私有运行环境；
- OpenCode 优先，Pi/Codex 在实际验证前只列为目标集成。

底层能力大多可由 Docker、volume 和脚本组合得到。tinybox 的价值必须由更短的
本地 Agent 接入链路和可复现实测证明，而不能宣称底层容器原语新颖。

## 2. 相邻方案

| 方案 | 主要形态 | 与 tinybox 的关系 | 最值得学习 |
|---|---|---|---|
| Anthropic Sandbox Runtime | 本地 OS 原语，Linux 使用 bubblewrap，网络经代理 | 最直接竞品 | 命令 wrapper、默认拒绝网络、FS+网络必须组合、MCP 接入 |
| Docker Sandboxes | Coding Agent 运行在隔离 VM 中，提供 Agent 专门集成 | 更强外层边界 | OpenCode 开箱集成、host secret store + placeholder、网络 policy UX |
| E2B | 面向 Agent 的按需 Firecracker microVM 云服务 | 云端替代/外层 | 极简 SDK、template、sandbox controller 认证、生命周期 API |
| Daytona | Agent 基础设施与 sandbox SDK/API | 云端替代/外层 | process/filesystem/Git API、snapshot、凭据注入代理 |
| Modal Sandboxes | 云端不可信代码容器 API | 通用执行平台 | subprocess 风格 exec、流式 I/O、timeout、持久 ID、网络策略 |
| OpenCode custom tools | Agent 工具扩展与替换接口 | 直接集成面 | 同名覆盖 bash、项目级安装、permission 配合 |

### Anthropic Sandbox Runtime

官方将其定义为无需完整容器的轻量命令沙箱：Linux 使用 bubblewrap，文件系统
与网络限制作用于整个进程树，网络通过宿主 HTTP/SOCKS proxy，并明确指出只做
FS 或只做网络都不足够。它还能直接包装 MCP server。这与 tinybox 的目标最接近：
[官方仓库](https://github.com/anthropic-experimental/sandbox-runtime)。

tinybox 应学习：

- `srt <command>` 一样简单的 wrapper UX；
- 配置校验和清楚的默认规则；
- HTTP 与一般 TCP 的网络兼容性测试；
- violation 的即时用户反馈；
- 将 MCP server 视为高风险执行入口。

tinybox 的差异不应宣称“别人没有 Agent 沙箱”，而应聚焦 Rust/Linux 实现、
cgroup 资源约束、任务级审计以及可嵌入专用 runner。

### Docker Sandboxes

Docker 已提供 OpenCode 专门文档，并将 OpenCode 放入隔离沙箱。其 secret 模型
尤其值得借鉴：真实 key 留在宿主 secret store，沙箱只得到 placeholder，宿主
代理在批准的目标请求上替换；网络目标需要显式授权。
[Docker OpenCode 集成](https://docs.docker.com/ai/sandboxes/agents/opencode/)

这说明仅仅“不要把 key 放进沙箱”还不够，tinybox 后续应设计 credential broker，
而不是让 npm/Git/cloud token 作为普通环境变量进入 payload。

### E2B

E2B 把 Sandbox 定义为按需创建的隔离 Linux VM，并提供简单的 create/run SDK
与 template；官方说明其使用 Firecracker microVM。控制器通信默认启用 access
token，避免仅凭 sandbox ID 控制环境。
[E2B 文档](https://www.e2b.dev/docs)、[secured access](https://e2b.dev/docs/sandbox/secured-access)

tinybox 应学习其最小 SDK 契约和控制器认证；不应与其争夺恶意跨租户 workload。

### Daytona

Daytona 提供 sandbox 生命周期、进程、文件、Git 和 snapshot API，并把 sandbox
daemon 放在隔离环境中。其凭据方案使用 opaque placeholder，由出口代理只在批准
目标上替换真实凭据。
[Daytona 文档](https://www.daytona.io/docs/en/)、[Secrets](https://www.daytona.io/docs/en/secrets/)

tinybox 应学习其 toolbox API 和持久 workspace/snapshot 模型，但保持本地小型
runtime 范围，不扩展成完整云控制面。

### Modal Sandboxes

Modal 的 Sandbox API 接近异步 subprocess：create、exec、stdout/stderr、timeout、
ID 恢复和 terminate，并支持完全断网、CIDR 或域名出口限制。
[Sandbox API](https://modal.com/docs/guide/sandboxes)、[网络策略](https://modal.com/docs/guide/sandbox-networking)

tinybox 的 Agent adapter 应复制这种清晰的进程语义，而不是暴露容器实现细节。

### OpenCode

OpenCode 支持项目或全局 custom tool；同名自定义 `bash` 可以覆盖内置 `bash`。
它还提供工具级 allow/ask/deny 权限。这给 tinybox 一个无需 fork OpenCode 的稳定
接入方向：[Custom Tools](https://opencode.ai/docs/custom-tools)、
[Tools](https://opencode.ai/docs/tools)。

权限提示是用户意图层，tinybox 是内核强制层，两者互补，不能用其中之一冒充另一个。

## 3. 对路线图的直接影响

1. 先收稳 persistent task、subprocess 风格 exec 和异常退出清理。
2. 建立 base/rootfs、private home/cache、workspace 和 volume 的环境模型。
3. 把 OpenCode adapter 和命令语义兼容放在首个集成里程碑。
4. Pi/Codex 分别验证扩展接口和整 Agent 包装，不能预先声称透明支持。
5. save/restore 只作为核心 MVP 后的可选对照实验，不自动成为主功能。
6. 测量安装步骤、cold/warm 延迟、RSS 和磁盘增量后，再使用
   “轻量、快速”性能表述。
7. 与 microVM 产品比较时承认安全边界不同：tinybox 可部署在它们内部，而不是
   声称以同内核机制达到同等隔离。

## 4. 不应采用的叙事

- “市面上没有面向 Agent 的沙箱”；
- “runc 为可信服务设计，因此不适合不可信代码”；
- “组合更多 Linux 原语就等于 microVM 安全强度”；
- “只替换 bash 就保护了整个 OpenCode”；
- “域名 allowlist 自动解决凭据和重定向外泄”；
- “动态 phase 是用户选择 tinybox 的首要原因”。
- “tinybox checkpoint 是新的完整容器 CR”；
- “Docker/E2B 很重，所以 tinybox 必然更快”；
- “Pi Agent/Codex 已经支持”（在专用 smoke test 通过之前）。
