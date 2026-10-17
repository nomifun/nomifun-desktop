# Linux 正式构建与安装验收（2026-10-17）

本页是正式构建与验收后的补充记录，不在下述私有源码快照提交内。实现代码、脚本及测试来自已审查的 91 文件 Linux 变更；原 WSL main 和未提交改动保留，没有推送或发布。它补齐[总验证记录](global-os-linux-2026-10-17.zh.md)中先完成的本地 debug 包证据，不改写旧报告。

## 源码与正式入口

私有构建目录为 `/home/muri/nomifun-linux-evidence-20261017/formal-build/run-20261017T051859Z-9d0532cb`，源码在其 `source` 子目录。Git 分支为 `codex/linux-local-release-validation`，提交 `298d61cc9ed5691ea9561a5fdd0c4642fc23319c`，tree `3314a0b5257907fe449b734f0dd0f38fdb27fc07`，来自 BASE `d1e968bfe262c39deeddcc460f3df27ab26ff0f7` 及首批六文件审查包。全部 55 tracked + 36 新增文件在工作树、私有 index 与提交逐字节/模式核对。仅私有快照沿用当前 Windows NomiFun 仓库已配置的贡献者身份；没有修改原 WSL/global Git 配置或绕过签名要求。

实际以 UID1000 执行 `bun install --frozen-lockfile` 与 `bun run build:linux x64 -- --bundles deb`，两者退出0。正式入口于 05:19:35Z–05:50:44Z 完成，使用仓库 release profile 的 opt3、Thin LTO、codegen-units1；target、build.noindex、node_modules 和 prune 全部在快照内。1790 个依赖 symlink 的检查未发现指向原工作树。构建前后原仓库 HEAD/main、index、status、config 和 91 个源文件一致；证据为 `source-snapshot-report.json`、`node-modules-isolation.json` 和 `original-checkout-preservation-final.json`。

## 正式产物与锁

| 项目 | SHA-256 / 结果 |
| --- | --- |
| `source/dist/desktop/NomiFun_0.8.12_amd64.deb` | `6b29c8332fb473666cafc35a267826c1abdfab6870dba362c79e133241719900`，100032346 bytes |
| Cargo Host（包类型标记前） | `4c2e156f5ee2472c65fb6d237791c485588a12418dbab2f53253ec09e03a841f` |
| 包内及实际安装 `/usr/bin/nomifun-desktop` | `b4a9306dcd6d12e59370a6225250b17a2e90c70e443b57cb4af0271690ae6cd2`，220788744 bytes |
| release runner | `e9ea8e967ad84e30541545f5f8b346920d7d38b17d8152797c93898c2777523e`，stage `run-gwAg82` |
| runner/App build identity | `cf41ea5eefe8741cbd44a86fbd5175a6d711cb6597332eafd893b4c11b3a8ada` |
| 自动 Host/package/LICENSE/NOTICE lock | `9127c3403179a504a04c2127ffb651dc85a8f10497108215a48420d23eff1fef`，官方 verify通过 |
| 补充实际 runner/policy 的 complete lock | `91745666d0397e21d4d9693677cccdd5421339ab632a35abf9134cb75fe2580a`，官方六项 verify通过 |

Host 全文件比较仅有 offset46988656 的 Tauri 标准 `UNK`→`DEB` 三字节变化，不将两侧 SHA 混为相同。包内 runner/policy 与 stage 全字节一致，16 个 regular payload 均 root:root、无 setuid/setgid，runner0755/policy0644；control 只有 control/md5sums，无 maintainer scripts 或 conffiles，本机依赖原生检查通过。自动 lock 的范围不含独立 helpers，因此保留自动锁并通过同一官方工具另建 complete lock，没有修改生产脚本伪造范围。

`release-runner-native.json` 是优化 runner 的真实独立 root 协议验收，全部通过：受保护普通拒绝/管理员读写、UID/HOME/literal argv、scope/source worker、并发取消、超时、16 MiB 控制输出、EOF 与失败回执的确切进程回收。它不声称 polkit 用户认证；真实 polkit 产品授予见总记录中的独立报告。

## 实际安装与 release 产品

原生 dpkg 安装历史 debug 包，再由正式 release 包完成 0.8.12 over 0.8.12 同版本替换；两次 `dpkg --verify` 无差异，关键组件摘要/owner/mode/包归属匹配。普通用户 App273467 的 `/proc/exe` 是实际 `/usr/bin/nomifun-desktop`、UID1000，精确 inode/device/hash、私有数据库 fd、40461 listener 及 `/health`200均验证。NOMIFUN_DATA_DIR 与 XDG 全部隔离，真实 HOME 为 `/home/muri`；无预建 DB，由实际 App coordinator 引导。owner 为独立 fixture preparation，JWT 来自实际 App HTTP login，不把 fixture setup 当作 native setup ingress 验收。

| 正式 release 产品入口 | 结果 / 报告 SHA-256 |
| --- | --- |
| Minimal missing-default | 16 checks / 14 正式步骤、两 Turn，通过；`0985b86ef894e29f5e43314e867616b8ff997dfc050ec8d5b759b7a3e6522c9b` |
| General files/process | 21 checks / 17 正式步骤，通过；`2f2ea7471d1a665f22dbb36ad077f6e7c456698a754174be79e3c8bf6798da89` |
| 两项汇总 | `de90fd1a73ec51adbc7f35646f3a09ce845d979753de6a27adfc800ef24a03c5` |

Minimal Session `01a1486b-1b5c-7113-bb9e-ddf543e65531` 的两个 Turn 整份 frozen binding SHA 均为 `5670332af6493a64ed5bdf64dbe5d720becd98043e1f0166818242ed250bd0be`。删除默认目录后持续缺失、不恢复或换绑，合法绝对文件操作、外部 cwd 进程完整 start/poll/cancel/reaped/terminal/PIDgone通过。General Session `01a1486b-8392-7cb1-bcd4-0d42be872cf0` 覆盖外部六项文件/合法父路径/symlink/cwd/取消清理。两项 completed、cleanup proven、pending0/unknown0、completion delivered；PID274036/274190与两个产品夹具均消失，没有 root/TTY/新管理员口令要求。

原报告的 byte-exact 持久副本在 `/home/muri/nomifun-linux-evidence-20261017/installed-release-product`，仅复制报告、identity、汇总与 SHA 清单，不含凭据/JWT/数据库。App273467 最后 SIGTERM 正常 exit0；原生卸载后16项payload全部消失，再恢复此前手动 RT4iTT helper/policy（a88d9522…/edd1124f…，root:root、0755/0644、无dpkg归属）。全过程证据为 formal run 目录的 `package-install-replace-result.json`、`installed-release-host-health.json` 和 `package-remove-restore-result.json`。

Node 后继 Turn 与重启补验另外使用同一个正式 `.deb` 重新安装，在实际 `/usr/bin` release Host 上通过。默认 Minimal Session `01a14881-747e-71c3-8c68-774082e6dafa` 的 3 个真实 Turn / 9 步完成离线 npm 安装/postinstall、CLI literal argv、后继 Turn 不重装调用、App277137 exit0→App278572 同环境重启、原 JWT/owner/冻结 binding、同一 CLI 字节/inode/mode0700 保持及最后卸载/夹具删除。报告 SHA-256 为 `02170f4281104130d6bb0608f7cd72f157db2fa20f791d497ac80672a0647e16`，完整边界见[Node 验证](global-os-linux-node-2026-10-17.zh.md)。本 passed 主体为 verified-cache reuse；独立第三轮真实 managed publication 与 canonical returned Effect 另有原件，不把 early failed 结果改成 passed。新增包周期记录为 formal run 内 `node-package-install-replace-result.json` 和 `node-package-remove-restore-result.json`；末态与进程回收由仓库外 `DELIVERY-FINAL.json` 复核。此次补验仅修改 post-build 文档与仓库外 QA，未修改该正式包的生产源码。

本页只证明当前 Ubuntu26.04/x86_64 的正式 release-profile deb 构建及本地验收，不等于公开发行、跨版本升级、其他发行版/CPU、RPM/AppImage 原生安装、完整 Wayland/compositor/物理设备矩阵。此前管理员、X11 AX/clipboard 和媒体报告仍保持各自真实 Host 身份，没有改写成在本 release Host 上重新执行。
