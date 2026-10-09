#!/usr/bin/env bash
# ============================================================================
# 出「带 Developer ID 签名 + 公证」的 macOS 安装包。
#
#   bun run build:signed          # 等价于带签名的 build
#   bun run build:signed --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                         # 额外产出 updater 的 .sig(需另配 updater 密钥)
#
# 密钥/口令全部来自本地 apps/desktop/signing/.env.signing(已 gitignore,绝不入库)。
# 该文件不存在时直接报错并提示如何创建。
# ============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The signed entry uses exactly the same final App and distribution artifacts.
exec bash "$SCRIPT_DIR/desktop-build-mac.sh" --signed "$@"
