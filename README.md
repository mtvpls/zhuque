# 朱雀 (Zhuque)

<div align="center">

一个轻量级、高性能的定时任务管理平台

[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-18-blue.svg)](https://reactjs.org/)
[![Docker](https://img.shields.io/badge/Docker-ghcr.io-blue.svg)](https://ghcr.io/mtvpls/zhuque)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

</div>

## 📸 界面预览

<div align="center">

![界面截图1](screenshot/1.png)

![界面截图2](screenshot/2.png)

</div>

## ✨ 特性

- 🚀 **高性能** - 基于 Rust + Axum 构建，内存占用低，响应速度快
- 📅 **定时任务** - 支持 Cron 表达式，灵活配置任务执行时间
- 📝 **脚本管理** - 在线编辑、上传、执行 Python/Node.js/Shell 脚本
- 📦 **依赖管理** - 统一管理 Python、Node.js、Linux 系统依赖
- 🔐 **安全认证** - JWT Token 认证 + TOTP 二次验证，保护 API 接口
- 📊 **日志查看** - 实时查看任务执行日志，支持搜索和过滤
- 💻 **Web 终端** - 基于 WebSocket 的在线终端，支持实时交互和窗口调整
- 🔄 **订阅管理** - 支持 Git 仓库订阅，定时拉取更新并执行脚本
- 🎨 **现代化 UI** - 基于 Arco Design，响应式设计，支持移动端

## 🏗️ 技术栈

### 后端
- **Rust** - 系统编程语言
- **Axum** - Web 框架
- **SQLite** - 数据库
- **Tokio** - 异步运行时
- **Tokio-cron-scheduler** - 定时任务调度

### 前端
- **React 18** + **TypeScript**
- **Vite 5** - 构建工具
- **Arco Design** - UI 组件库
- **Monaco Editor** - 代码编辑器
- **Zustand** - 状态管理
- **ECharts** - 数据可视化

## 📦 快速开始

### 使用 Docker（最简单）

使用预构建的 Docker 镜像快速启动：

```bash
docker run -d \
  --name zhuque \
  -p 3000:3000 \
  -v $(pwd)/data:/app/data \
  -e TZ=Asia/Shanghai \
  ghcr.io/mtvpls/zhuque:latest
```

首次访问 `http://localhost:3000` 会自动跳转到初始设置页面，设置管理员账号和密码。

或使用 Docker Compose：

```bash
docker-compose up -d
```

### 开发环境

前置要求：
- Rust 1.70+
- Node.js 18+
- SQLite 3

1. 克隆项目
```bash
git clone <repository-url>
cd xuanwu
```

2. 启动后端
```bash
cargo run
```

后端服务运行在 `http://localhost:3000`

3. 启动前端
```bash
cd web
npm install
npm run dev
```

前端服务运行在 `http://localhost:5173`

### 生产部署

#### 方式一：Docker 镜像（推荐）

```bash
docker run -d \
  --name zhuque \
  -p 3000:3000 \
  -v $(pwd)/data:/app/data \
  -e RUST_LOG=info \
  -e TZ=Asia/Shanghai \
  --restart unless-stopped \
  ghcr.io/mtvpls/zhuque:latest
```

> **首次启动：** 访问 `http://localhost:3000` 会自动跳转到初始设置页面，请设置管理员账号和强密码。

#### 方式二：Docker Compose

```bash
docker-compose up -d
```

#### 方式三：手动部署

1. 构建后端
```bash
cargo build --release
```

2. 构建前端
```bash
cd web
npm run build
```

3. 设置环境变量（可选，创建 `.env` 文件或直接导出）
```bash
export RUST_LOG=info
export TZ=Asia/Shanghai
```

4. 运行
```bash
./target/release/zhuque
```

后端会自动服务前端静态文件。

> **首次启动：** 访问 `http://localhost:3000` 会自动跳转到初始设置页面，请设置管理员账号和强密码。

## 📁 项目结构

```
xuanwu/
├── src/                    # Rust 后端源码
│   ├── api/               # API 路由处理
│   ├── middleware/        # 中间件（认证等）
│   ├── models/            # 数据模型
│   ├── scheduler/         # 任务调度器
│   ├── services/          # 业务逻辑
│   ├── utils/             # 工具函数
│   └── main.rs           # 入口文件
├── web/                   # React 前端
│   ├── src/
│   │   ├── api/          # API 接口封装
│   │   ├── components/   # 公共组件
│   │   ├── layouts/      # 布局组件
│   │   ├── pages/        # 页面组件
│   │   ├── router/       # 路由配置
│   │   ├── stores/       # 状态管理
│   │   └── utils/        # 工具函数
│   └── package.json
├── data/                  # 数据目录
│   ├── db/               # 数据库文件
│   └── scripts/          # 脚本文件
├── scripts/              # 部署脚本
├── Cargo.toml           # Rust 项目配置
├── Dockerfile           # Docker 镜像
└── docker-compose.yml   # Docker Compose 配置
```

## 🎯 核心功能

### 定时任务管理
- ✅ 创建、编辑、删除任务
- ✅ Cron 表达式支持
- ✅ 任务启用/禁用
- ✅ 立即执行任务
- ✅ 任务分组管理
- ✅ 开机自动执行

### 脚本管理
- ✅ 文件树展示
- ✅ 在线代码编辑（Monaco Editor）
- ✅ 语法高亮（Python/Node.js/Shell）
- ✅ 脚本上传/下载
- ✅ 在线调试执行
- ✅ 目录管理

### 依赖管理
- ✅ Python3 依赖（pip）
- ✅ Node.js 依赖（npm）
- ✅ Linux 系统依赖（apt）
- ✅ 批量安装
- ✅ 重新安装
- ✅ 安装状态实时显示

### 环境变量管理
- ✅ 键值对管理
- ✅ 环境变量分组
- ✅ 任务级环境变量

### 执行日志
- ✅ 任务执行日志查看
- ✅ 实时日志流
- ✅ 日志搜索和过滤
- ✅ 日志清理

### Web 终端
- ✅ 基于 WebSocket 的实时终端
- ✅ 支持终端窗口大小调整
- ✅ 自动加载环境变量
- ✅ PTY 伪终端支持
- ✅ 完整的 Shell 交互体验

### 订阅管理
- ✅ Git 仓库订阅
- ✅ 定时拉取更新（Cron 表达式）
- ✅ 分支选择
- ✅ 自动执行订阅脚本
- ✅ 订阅启用/禁用
- ✅ 执行状态和日志记录

### 系统配置
- ✅ 镜像源配置（pip/npm）
- ✅ 数据备份与恢复
- ✅ JWT 认证系统
- ✅ TOTP 二次验证（支持 Google Authenticator 等验证器）
- ✅ 备用恢复码

## 🔧 配置说明

### 环境变量

| 变量名 | 说明 | 默认值 | 必填 |
|--------|------|--------|------|
| `JWT_SECRET` | JWT 密钥（建议自定义） | 自动生成 UUID | 否 |
| `WEBHOOK_TOKEN` | Webhook 认证令牌 | 无 | 否* |
| `DATA_DIR` | 数据目录路径 | `./data` | 否 |
| `PORT` | 服务端口 | `3000` | 否 |
| `RUST_LOG` | 日志级别 (trace/debug/info/warn/error) | `info` | 否 |
| `TZ` | 时区设置 | `Asia/Shanghai` | 否 |
| `AUTO_RESTORE_ON_STARTUP` | 启动时自动恢复备份 | `false` | 否 |
| `WEBDAV_URL` | WebDAV 服务器地址 | 无 | 否** |
| `WEBDAV_USERNAME` | WebDAV 用户名 | 无 | 否** |
| `WEBDAV_PASSWORD` | WebDAV 密码 | 无 | 否** |
| `WEBDAV_REMOTE_PATH` | WebDAV 远程路径 | 无 | 否 |

> **注意：**
> - `WEBHOOK_TOKEN` 如果需要使用 Webhook 功能则必须配置
> - `WEBDAV_*` 如果启用 `AUTO_RESTORE_ON_STARTUP` 则必须配置 WebDAV 相关信息
> - ~~`AUTH_USERNAME` 和 `AUTH_PASSWORD` 已废弃~~，首次启动时通过 Web 界面设置管理员账号

**Docker 运行示例：**

```bash
docker run -d \
  --name zhuque \
  -p 3000:3000 \
  -v $(pwd)/data:/app/data \
  -e JWT_SECRET=your_jwt_secret_key \
  -e WEBHOOK_TOKEN=your_webhook_token \
  -e DATA_DIR=/app/data \
  -e PORT=3000 \
  -e RUST_LOG=info \
  -e TZ=Asia/Shanghai \
  --restart unless-stopped \
  ghcr.io/mtvpls/zhuque:latest
```

**重要提示：**
- 首次启动时，访问 Web 界面会自动跳转到初始设置页面，请设置管理员账号和强密码
- 如需使用 Webhook 功能，必须配置 `WEBHOOK_TOKEN`

**本地开发示例：**

创建 `.env` 文件（可选）：

```bash
DATABASE_URL=sqlite://./data/db/zhuque.db
RUST_LOG=debug
TZ=Asia/Shanghai
```

### 默认端口

- 后端 API: `3000`
- 前端开发服务器: `5173`

### 数据持久化

使用 Docker 时，建议挂载 `data` 目录以持久化数据：

```bash
-v $(pwd)/data:/app/data
```

该目录包含：
- `data/db/` - SQLite 数据库文件
- `data/scripts/` - 用户上传的脚本文件
- `data/logs/` - 任务执行日志

## 📖 API 文档

详细的 API 文档请参考 [ARCHITECTURE.md](ARCHITECTURE.md)

主要接口：
- `/api/auth/login` - 用户登录（第一步）
- `/api/auth/totp/verify` - TOTP 验证（第二步）
- `/api/auth/totp/*` - TOTP 管理接口
- `/api/tasks` - 任务管理
- `/api/scripts` - 脚本管理
- `/api/env` - 环境变量管理
- `/api/dependences` - 依赖管理
- `/api/logs` - 日志管理
- `/api/configs` - 系统配置
- `/api/terminal` - Web 终端连接（WebSocket）
- `/api/subscriptions` - 订阅管理

## 🛠️ 开发指南

### 后端开发

```bash
# 运行开发服务器
cargo run

# 运行测试
cargo test

# 代码格式化
cargo fmt

# 代码检查
cargo clippy
```

### 前端开发

```bash
cd web

# 安装依赖
npm install

# 开发服务器
npm run dev

# 构建生产版本
npm run build

# 代码检查
npm run lint
```

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

## 📄 许可证

[MIT License](LICENSE)

## 🙏 致谢

本项目受 [青龙面板](https://github.com/whyour/qinglong) 启发。

---

**注意**: 本项目仅供学习交流使用，请勿用于非法用途。
