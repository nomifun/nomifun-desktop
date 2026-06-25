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
ENV_FILE="$ROOT/apps/desktop/signing/.env.signing"

if [[ ! -f "$ENV_FILE" ]]; then
  cat >&2 <<EOF
❌ 找不到本地签名配置: $ENV_FILE

请先创建它(不会入库):
  cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
然后按文件内注释 / apps/desktop/signing/README.md 填入你的签名 + 公证信息。
EOF
  exit 1
fi

# 加载本地密钥环境变量(set -a 让 source 进来的变量自动 export 给子进程)
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

# notarytool 要求 .p8 用绝对路径;这里把相对仓库根的路径补成绝对路径,方便填写。
if [[ -n "${APPLE_API_KEY_PATH:-}" && "${APPLE_API_KEY_PATH:0:1}" != "/" ]]; then
  export APPLE_API_KEY_PATH="$ROOT/$APPLE_API_KEY_PATH"
fi

# ── 基本校验:必须有签名身份 ───────────────────────────────────────────────
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" && -z "${APPLE_CERTIFICATE:-}" ]]; then
  echo "❌ 既没设 APPLE_SIGNING_IDENTITY,也没设 APPLE_CERTIFICATE,无法签名。" >&2
  exit 1
fi

# ── 提醒:没配公证只能解决一半 ─────────────────────────────────────────────
HAS_NOTARY=0
if [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY_PATH:-}" ]]; then
  HAS_NOTARY=1
  if [[ ! -f "$APPLE_API_KEY_PATH" ]]; then
    echo "❌ 找不到 App Store Connect API Key: $APPLE_API_KEY_PATH" >&2
    exit 1
  fi
  if [[ "$APPLE_API_KEY_PATH" != *.p8 ]]; then
    echo "❌ APPLE_API_KEY_PATH 必须指向 AuthKey_*.p8,当前是: $APPLE_API_KEY_PATH" >&2
    exit 1
  fi
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
  HAS_NOTARY=1
fi
if [[ "$HAS_NOTARY" -eq 0 ]]; then
  echo "⚠️  未配置公证(notarization)变量:会签名但不公证。" >&2
  echo "    别人下载后仍会被 Gatekeeper 拦(提示「无法验证开发者」)。" >&2
fi

echo "▶ 签名身份: ${APPLE_SIGNING_IDENTITY:-(用 .p12: APPLE_CERTIFICATE)}"
[[ "$HAS_NOTARY" -eq 1 ]] && echo "▶ 公证: 已启用,构建末尾会自动提交 Apple 公证并 staple"
echo

submit_for_notarization() {
  local artifact="$1"

  if [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY_PATH:-}" ]]; then
    xcrun notarytool submit "$artifact" \
      --key "$APPLE_API_KEY_PATH" \
      --key-id "$APPLE_API_KEY" \
      --issuer "$APPLE_API_ISSUER" \
      --wait
  else
    xcrun notarytool submit "$artifact" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
  fi
}

notarize_dmg_artifacts() {
  if [[ "$(uname -s)" != "Darwin" || "$HAS_NOTARY" -eq 0 ]]; then
    return
  fi

  local dmg_dir="$ROOT/target/release/bundle/dmg"
  if [[ ! -d "$dmg_dir" ]]; then
    return
  fi

  local found=0
  while IFS= read -r -d '' dmg; do
    found=1
    if xcrun stapler validate "$dmg" >/dev/null 2>&1; then
      echo "▶ DMG 已有公证票据: $dmg"
      continue
    fi

    echo "▶ 公证 DMG: $dmg"
    submit_for_notarization "$dmg"
    echo "▶ Staple DMG: $dmg"
    xcrun stapler staple "$dmg"
    xcrun stapler validate "$dmg"
  done < <(find "$dmg_dir" -maxdepth 1 -type f -name '*.dmg' -print0)

  if [[ "$found" -eq 0 ]]; then
    echo "ℹ️  未找到 DMG 产物,跳过 DMG 公证。"
  fi
}

# 复用既有的 build 脚本;额外参数透传(例如 --config 开 updater 产物)
bun run build "$@"
notarize_dmg_artifacts
