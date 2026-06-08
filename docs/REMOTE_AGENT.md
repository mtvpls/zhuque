# 远程机器接入说明

远程执行功能由服务端和 `zhuque-agent` 两部分组成：

- 服务端负责登记机器、下发指令、接收日志、查看状态和读取文件。
- 远程机器运行 `zhuque-agent`，注册后通过 WebSocket 长连接等待服务端指令。

## 1. 在远程菜单创建机器

远程机器先由控制面板创建，服务端会为该机器分配专属连接 token：

1. 登录 Zhuque。
2. 进入“远程机器”。
3. 点击“创建”。
4. 输入机器名称。
5. 复制页面生成的一键接入命令。

该命令包含 `agent_id` 和专属连接 token。token 只会在创建时展示一次，请在创建后立即复制。

## 2. 分发 agent

生产环境不要在目标机器临时拉源码构建。建议在 CI/CD 或可信构建机上构建 `zhuque-agent`，然后把同版本二进制分发到远程机器。

Windows 目标机器示例：

```powershell
New-Item -ItemType Directory -Force C:\zhuque
```

把 `zhuque-agent.exe` 放到：

```text
C:\zhuque\zhuque-agent.exe
```

Linux 目标机器示例：

```bash
sudo install -m 0755 zhuque-agent /usr/local/bin/zhuque-agent
```

开发或自编译时，可在源码目录构建：

```bash
cargo build --release --bin zhuque-agent --no-default-features
```

Windows 可执行文件通常在：

```text
target\release\zhuque-agent.exe
```

Linux/macOS 可执行文件通常在：

```text
target/release/zhuque-agent
```

## 3. 一条命令启动并上线

在远程机器执行控制面板生成的命令。示例：

Windows:

```powershell
C:\zhuque\zhuque-agent.exe start --server https://<server-host> --agent-id 1 --token <agent_token> --config C:\zhuque\zhuque-agent.json
```

Linux/macOS:

```bash
zhuque-agent start --server https://<server-host> --agent-id 1 --token <agent_token> --config /etc/zhuque-agent.json
```

`start` 会把 `agent_id/token` 保存到 `--config` 指定的文件，再自动建立长连接。以后重启同一条命令时会复用该配置文件。保持该进程运行。Web 页面进入“远程机器”后，机器状态应显示为 `online`。

## 4. 生产环境常驻运行

生产环境应使用进程守护，不建议手动开一个终端长期挂着。

可以直接把一键接入命令放进 systemd、Windows 服务或进程守护。首次启动会写入配置文件，后续重启会复用同一个 agent。

Linux systemd 示例：

```ini
[Unit]
Description=Zhuque Remote Agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/zhuque-agent start --server https://<server-host> --agent-id 1 --token <agent_token> --config /etc/zhuque-agent.json
WorkingDirectory=/opt/zhuque-agent
Environment=ZHUQUE_AGENT_ALLOWED_ROOTS=/opt/scripts;/var/log/zhuque
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Windows 可用 NSSM、WinSW 或计划任务把 `zhuque-agent.exe start ...` 注册为常驻服务。

## 5. 文件查看权限

agent 默认只允许查看当前工作目录下的文件。需要放开其他目录时，在远程机器启动 agent 前设置：

Windows PowerShell:

```powershell
$env:ZHUQUE_AGENT_ALLOWED_ROOTS="D:\scripts;D:\logs"
.\zhuque-agent.exe start --server https://<server-host> --agent-id 1 --token <agent_token> --config .\zhuque-agent.json
```

Linux/macOS:

```bash
export ZHUQUE_AGENT_ALLOWED_ROOTS="/opt/scripts;/var/log/zhuque"
zhuque-agent start --server https://<server-host> --agent-id 1 --token <agent_token> --config /etc/zhuque-agent.json
```

多个目录用英文分号 `;` 分隔。单个文件读取上限为 10MB。

## 6. 页面使用

进入 Web 页面 `远程机器`：

1. 左侧或顶部选择在线机器。
2. 在“执行”里下发命令或执行远程脚本。
3. 在“日志”里查看实时输出，也可以终止当前命令。
4. 在“文件”里打开目录或读取文本文件。
5. 在“状态”里刷新 CPU、内存、运行时长等状态。

定时任务页面也可以选择“远程机器”作为执行目标。

## 常见问题

### 启动失败：invalid --agent-id

不要手写占位符。请在“远程机器”页面点击“创建”，复制系统生成的完整命令。

### 启动失败：invalid agent token

远程机器命令里的 `--token` 和该机器创建时分配的 token 不一致。重新创建机器并复制新命令。

### 页面有机器但一直 offline

已经创建机器，但没有运行 `zhuque-agent start`，或 WebSocket 连接被反向代理阻断。反向代理需要支持 WebSocket Upgrade。

### 文件读取提示 path is outside allowed roots

目标路径不在 `ZHUQUE_AGENT_ALLOWED_ROOTS` 允许范围内。调整该环境变量后重启 agent。
