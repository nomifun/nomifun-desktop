# Linux Web 服务与 systemd

这里保存无图形界面的 `nomifun-web` 服务模板；Tauri 的 DEB/AppImage 桌面安装包是另一种交付。
Docker、HTTPS、管理员初始化和共享部署配置统一见
[Web 服务器部署](../../docs/guides/web-server-deployment.zh.md)，本页只保留 systemd 操作。

## 构建与部署布局

在 Linux 构建机执行：

```bash
bun install
bun run build:ui
cargo build --release -p nomifun-web
sudo useradd --system --home /var/lib/nomifun --shell /usr/sbin/nologin nomifun
sudo mkdir -p /opt/nomifun/web
sudo cp target/release/nomifun-web /opt/nomifun/
sudo cp -r ui/dist/. /opt/nomifun/web/
```

服务布局与 [nomifun-web.service](nomifun-web.service) 一致：

| 路径 | 用途 |
| --- | --- |
| `/opt/nomifun/nomifun-web` | Web 宿主程序 |
| `/opt/nomifun/web/` | 构建后的 SPA |
| `/var/lib/nomifun/` | 由 `StateDirectory=nomifun` 创建、服务用户持有的数据与缓存 |

## 安装服务

```bash
sudo cp packaging/linux/nomifun-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nomifun-web
sudo systemctl status nomifun-web
journalctl -u nomifun-web -f
```

模板以专用 `nomifun` 用户运行，默认监听 `127.0.0.1:8787`。先完成管理员初始化或预置管理员，
再按部署指南开放访问；公网通过 TLS 代理，配置 `NOMIFUN_HTTPS=true`。
模板中的 `NOMIFUN_DATA_DIR=/var/lib/nomifun` 必须与 `StateDirectory=nomifun` 保持一致，
`NOMIFUN_WEB_DIST` 指向实际 SPA 目录。修改 unit 后执行 `daemon-reload` 并重启服务。
参数与环境变量的唯一说明见[配置参考](../../docs/reference/configuration.zh.md)。

## 运行时组件与诊断

OfficeCLI 使用产品明确指定的受管组件入口；普通 Node/Bun 用户命令按服务用户环境发现，
Bun/bunx 可回退到既有有效受管缓存。普通发现不自动下载缺失工具。组件准入、缓存与
用户命令的区别遵循[外部工具合同](../../docs/architecture/external-tools.zh.md)。
需要组件诊断时额外构建 `nomicore`；`nomifun-web` 不接受 `tool-doctor` 子命令：

```bash
cargo build --release -p nomifun-app --bin nomicore
sudo cp target/release/nomicore /opt/nomifun/
sudo -u nomifun /opt/nomifun/nomicore --data-dir /var/lib/nomifun tool-doctor --verify
```

`--verify` 校验本地可用组件，可能解压内嵌 OfficeCLI 并执行私有探测，不下载缺失的
Node/Bun，也不打开数据库。仅查看供应清单时不加该参数。
服务更新前停止进程并备份完整数据集，恢复与版本兼容边界见[数据与存储](../../docs/architecture/data-and-storage.zh.md)。
