# OpenClaw 安装指南

## 系统要求

- **Node.js**: 版本 ≥ 22.19（必需）；Node 24 是当前推荐运行时。
- **操作系统**: macOS、Linux、Windows（原生 Windows Hub / PowerShell installer / WSL2 Gateway 均可）。
- **包管理器**: npm、pnpm 或 bun。Gateway 生产运行仍推荐 Node；bun 适合作为 CLI 全局安装路径。

## 检查 Node.js 版本

```bash
node --version
```

如果版本低于 22.19，需要先升级 Node.js；能升级到 Node 24 时优先使用 Node 24。

## 安装方式

### 方式 1：使用官方安装脚本（推荐）

**macOS/Linux:**

```bash
curl -fsSL https://openclaw.ai/install.sh | bash
```

**Windows (PowerShell):**

```powershell
iwr -useb https://openclaw.ai/install.ps1 | iex
```

### 方式 2：npm 全局安装

```bash
npm install -g openclaw@latest
openclaw onboard --install-daemon
```

或使用 pnpm：

```bash
pnpm add -g openclaw@latest
pnpm approve-builds -g
openclaw onboard --install-daemon
```

或使用 bun：

```bash
bun add -g openclaw@latest
openclaw onboard --install-daemon
```

### 方式 3：从源码构建（开发）

```bash
git clone https://github.com/openclaw/openclaw.git
cd openclaw
pnpm install
pnpm ui:build  # 首次运行会自动安装 UI 依赖
pnpm build
```

## 验证安装

```bash
openclaw --version
```

## 安装后下一步

安装完成后，运行新手引导向导：

```bash
openclaw onboard --install-daemon
```

这会引导你完成 Gateway 配置、模型认证、渠道设置等。
