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
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# 签名环境装载 / 校验 / 公证函数统一在公共库,与 build:mac --signed 共用一份实现
# shellcheck source=lib/mac-signing.sh
source "$SCRIPT_DIR/lib/mac-signing.sh"

load_signing_env "$ROOT"
require_signing_identity
detect_notary

echo "▶ 签名身份: ${APPLE_SIGNING_IDENTITY:-(用 .p12: APPLE_CERTIFICATE)}"
[[ "$HAS_NOTARY" -eq 1 ]] && echo "▶ 公证: 已启用,构建末尾会自动提交 Apple 公证并 staple"
echo

# 复用既有的 build 脚本;额外参数透传(例如 --config 开 updater 产物)
bun run build "$@"
notarize_dmg_dir "$ROOT/target/release/bundle/dmg"
