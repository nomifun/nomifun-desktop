# R16 审计记录：代理解析与跨平台装配

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R16-01：环境编码不能使代理查询 panic

用独立测试子进程注入一个无关的非 Unicode 环境变量，旧 process_env_proxy_names 因
std::env::vars 解码 panic。修复使用 vars_os 并忽略不可解码条目；保持大小写键名和空值规则。
未修改当前测试进程或用户的实际环境变量；Windows 子进程使用隐藏窗口标志。

## R16-02：端点与装配规则

畸形 [::1]garbage 旧实现被接受，回归先失败后通过。IPv6 括号内必须是真实 IPv6，
括号后只能为空或合法端口；端口 0 统一拒绝，删除 Linux 独有的重复端口检查。

将 macOS、Windows、GNOME、KDE 四处相同的 HTTP/HTTPS 优先、SOCKS 回退、空配置
拒绝和 NO_PROXY 装配合并为 SystemProxyConfig::detected。删除永远不会为空的默认
NO_PROXY 列表判空，以及重复的默认 scheme 分支。保持既有 bypass 策略，不擅自
把 WinINET 的 <local> 扩展为其他主机匹配规则。

## 验证与接续

- 两个新增回归均红→绿；将两个已有 macOS 纯解析测试开放为跨平台测试。
- 代理定向 26/0，最终 nomifun-net 全量 47/0；进程边界与差异检查通过。
- 没有运行 macOS/Linux 原生系统探测；Windows 只验证解析/环境等测试，未改注册表。
- 代理辅助进程的退出后 read_to_end 和进程树归属仍待 R17；缓存并发和精确凭据脱敏
  仍待深入行为验证。此批不是整个网络模块完成。
