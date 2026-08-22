# OpenCode 裸奔与 tinybox 本地容器对照 Demo

> 状态：确定性 harness 已实现。`scripts/demo_local_agent.sh` 验证 bare/tinybox
> build/test、生命周期与度量；`scripts/test_m3.sh` 验证 OpenCode adapter contract
> 和真实加载。需要用户模型凭据的 live LLM prompt 仍是可选人工演示，不属于 CI
> 完成门。Demo 不是安全认证。

## 1. 演示结论

同一个单租户开发任务、同一个测试仓库、同一个 OpenCode 提示：

- 裸奔模式可以运行正常测试，但恶意脚本也能读取 workspace 外合成秘密、连接
  本地接收端并制造过量进程；
- tinybox 模式仍能完成正常构建和测试，同时拒绝这些行为并给出审计证据；
- 用户仍使用 OpenCode 的 `bash` 工具，不需要在 prompt 中书写容器命令。

## 2. 安全的 Demo fixture

脚本创建临时目录，退出时只删除这个精确目录：

```text
/var/tmp/tinybox-opencode-demo.<random>/
├── workspace/
│   ├── package.json
│   ├── src/
│   ├── test/
│   └── scripts/agent-attack.sh
├── outside/
│   └── synthetic-ssh-key       # 仅合成 canary
├── receiver/                   # 127.0.0.1 本地 HTTP fixture
├── policies/
│   ├── bare.json               # 只作标签，不交给 tinybox
│   └── protected.json
└── evidence/
```

绝不读取 `~/.ssh`、云 metadata 或公网地址。所谓“秘密读取”和“外泄”都指临时
目录内的合成 canary 与本地 receiver。

攻击脚本依次尝试：

1. 读取 `../outside/synthetic-ssh-key`；
2. 从 workspace 内的 symlink 读取该 canary；
3. 向未授权本地 receiver 发送 canary；
4. 连接 tinybox daemon/control endpoint；
5. 创建超过策略 `pids` ceiling 的子进程；
6. 写 workspace 外的合成目标。

每项输出机器可解析的 `ATTACK=name RESULT=...`，不能依赖 Agent 自述。

## 3. 两条运行路径

### A. 裸奔基线

OpenCode 使用原生 `bash`。演示提示固定为：

```text
Run the repository's demo verification command, fix the intentionally failing
unit test, rerun the tests, and report the command results.
```

正常测试通过后，确定性 verifier 直接调用仓库攻击脚本，证明它继承了 OpenCode
shell 相同的宿主权限。这里不要求模型主动攻击，避免模型随机性影响结果。

### B. tinybox 保护

项目级 `.opencode/tools/bash.ts` 覆盖内置 `bash`：

```text
OpenCode bash tool
  → tinybox-opencode-adapter(command, cwd, timeout)
  → orchestrator-owned task handle
  → tinybox exec/run
```

OpenCode 官方支持以同名 custom tool 替换内置工具，因此无需修改其核心：
[Custom Tools](https://opencode.ai/docs/custom-tools)。项目配置同时把其他可启动
进程的工具设为 deny/ask；OpenCode 的工具权限模型见
[Tools](https://opencode.ai/docs/tools)。

adapter 必须：

- 只接受 command/cwd/timeout，不接受 policy、rootfs、volume 或 network 参数；
- 从 Agent 不可写环境取得 task handle；
- 拒绝 workspace 之外的 cwd；
- 原样传递 stdout/stderr/exit code，支持取消与 timeout；
- tinybox 建立失败时拒绝执行，绝不 fallback 到裸 bash。

保护模式运行同一提示，并通过同一个 adapter 运行同一攻击 verifier。

## 4. 策略

第一版 Demo 使用静态任务策略，避免动态 phase 掩盖核心价值：

```json
{
  "version": 1,
  "filesystem": [
    {"path": "/workspace", "access": "read_write"},
    {"path": "/tmp", "access": "read_write"}
  ],
  "network": [],
  "resources": {
    "memory_bytes": 536870912,
    "cpus": 1.0,
    "pids": 32
  },
  "phases": []
}
```

宿主 workspace 映射到沙箱 `/workspace`。策略文件和 adapter 安装在 workspace
之外且不可写。第二版才增加“install 允许本地 package fixture，build 立即撤销”
作为独立演示，不与基础安全对照混在一起。

## 5. `read/edit` 的处理

只替换 `bash` 不能宣称 OpenCode 整体已被保护。真实 Demo 必须选择以下之一：

1. 用 `tinybox agent-host` 启动 OpenCode，使其整个进程只能读写 workspace 和
   必需的只读运行时路径；或
2. 暂时将 OpenCode 内置 `read/write/edit/apply_patch` 设为 deny，只提供经过
   workspace 校验的自定义工具。

首选方案 1，但需要先验收 OpenCode 的二进制、CA certificate、provider 配置和
环境凭据在最小 Landlock baseline 下仍能工作。不能为了兼容而开放整个 home。

formatter、LSP、MCP server 和插件也可能启动宿主进程；Demo 配置必须默认关闭，
逐个经过验收后才启用。

## 6. 展示顺序

1. 展示临时 fixture 和合成 canary，证明没有触碰真实秘密。
2. 裸奔运行正常测试：通过。
3. 裸奔运行 attack verifier：六项攻击展示可达结果。
4. 安装/启用 tinybox OpenCode adapter，不修改 prompt。
5. tinybox 运行正常测试：仍通过，输出与退出码匹配。
6. tinybox 运行同一 verifier：六项攻击全部被拒绝或受限。
7. 展示 policy hash 和结构化审计摘要。
8. 展示限制声明：rootful、同内核、不作为跨租户边界。

## 7. 验收矩阵

| 检查 | 裸奔预期 | tinybox 预期 | 证据 |
|---|---|---|---|
| 正常 workspace 读写 | 成功 | 成功 | 文件 hash |
| 单元测试 | 成功 | 成功 | stdout + exit 0 |
| workspace 外 canary | 成功 | 拒绝 | canary 未出现在输出 |
| symlink escape | 成功 | 拒绝 | errno + audit |
| 未授权 receiver | 成功 | 拒绝 | receiver access log |
| control API | 可达或依宿主配置 | 不可达 | socket result |
| 超量 fork | 无任务级限制 | pids ceiling 生效 | 最大子进程数 |
| workspace 外写 | 成功 | 拒绝 | 目标不存在 |
| 非零退出码 | 原值 | 相同原值 | adapter contract test |
| timeout/SIGINT | 终止 | 终止且无残留 | PID/cgroup 检查 |

任何保护项若因为缺少 root、Landlock 或 cgroup 被跳过，Demo 总结果必须是
`INCOMPLETE`，不能显示为成功。

## 8. 交付物

```text
integrations/opencode/
├── README.md
├── opencode.json
└── .opencode/tools/bash.ts
scripts/opencode-demo/
├── prepare.sh
├── run-bare.sh
├── run-protected.sh
├── verify-attacks.sh
└── cleanup.sh
tests/opencode_adapter.rs或独立 shell contract tests
```

第一阶段只实现确定性 adapter + harness；真实 LLM/OpenCode 运行作为可选演示，
因为它需要用户自己的模型凭据且输出具有随机性。CI 永远运行确定性路径。
