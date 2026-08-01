# ============================================================================
# macOS 签名/公证公共库 —— 被 desktop-build-signed.sh 与 desktop-build-mac.sh
# (--signed 分支) source 复用,消除两处整段双写。
#
# 用法(调用方需已定义仓库根目录并 set -euo pipefail):
#   source "$SCRIPT_DIR/lib/mac-signing.sh"
#   load_signing_env "$ROOT"        # 加载 .env.signing 并导出变量
#   require_signing_identity        # 校验签名身份,缺失则退出 1
#   detect_notary                   # 探测公证配置,设置全局 HAS_NOTARY(0/1)
#   notarize_dmg_dir <dir>          # 对目录下 DMG 逐个公证 + staple
#
# 本文件只定义函数,不产生副作用;非 macOS 环境 source 无害。
# ============================================================================

# 加载本地签名配置 apps/desktop/signing/.env.signing;$1 = 仓库根目录。
# set -a 让 source 进来的变量自动 export 给子进程(tauri build 据此自动签名)。
load_signing_env() {
  local root="$1"
  local env_file="$root/apps/desktop/signing/.env.signing"

  if [[ ! -f "$env_file" ]]; then
    cat >&2 <<EOF
❌ 找不到本地签名配置: $env_file

请先创建它(不会入库):
  cp apps/desktop/signing/.env.signing.example apps/desktop/signing/.env.signing
然后按文件内注释 / apps/desktop/signing/README.md 填入你的签名 + 公证信息。
EOF
    exit 1
  fi

  set -a
  # shellcheck disable=SC1090
  source "$env_file"
  set +a

  # notarytool 要求 .p8 用绝对路径;相对仓库根的路径补成绝对路径,方便填写。
  if [[ -n "${APPLE_API_KEY_PATH:-}" && "${APPLE_API_KEY_PATH:0:1}" != "/" ]]; then
    export APPLE_API_KEY_PATH="$root/$APPLE_API_KEY_PATH"
  fi
}

# 基本校验:必须有签名身份,否则退出 1。
require_signing_identity() {
  if [[ -z "${APPLE_SIGNING_IDENTITY:-}" && -z "${APPLE_CERTIFICATE:-}" ]]; then
    echo "❌ 既没设 APPLE_SIGNING_IDENTITY,也没设 APPLE_CERTIFICATE,无法签名。" >&2
    exit 1
  fi
}

# 探测公证配置,设置全局 HAS_NOTARY(1 = 可公证)。
# API key 三元组优先(顺带校验 .p8 存在且后缀正确),否则回退 APPLE_ID 三元组;
# 都没配则告警:会签名但不公证,下载方仍会被 Gatekeeper 拦。
detect_notary() {
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
}

# 提交单个产物给 Apple 公证并等待结果;$1 = 产物路径。
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

# 对指定目录下的 DMG 逐个公证 + staple(仅在 macOS 且配了公证时执行);$1 = 目录。
# 已带票据的 DMG 跳过;staple 会原地改写 DMG。
notarize_dmg_dir() {
  local dmg_dir="$1"
  if [[ "$(uname -s)" != "Darwin" || "${HAS_NOTARY:-0}" -eq 0 || ! -d "$dmg_dir" ]]; then
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
