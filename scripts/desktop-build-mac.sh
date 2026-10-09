#!/usr/bin/env bash
# ============================================================================
# 打 macOS 桌面端安装包(.dmg),并汇总到 dist/desktop/。仅能在 macOS 上运行。
#
#   bun run build:mac                 # 默认打 Apple Silicon arm64 DMG(不签名)
#   bun run build:mac --signed        # arm64 + Developer ID 签名 + 公证
#   bun run build:mac arm             # 显式选择 Apple Silicon
#   NOMIFUN_MACOS_DMG_FORMAT=UDZO bun run build:mac
#                                     # 兼容 zlib9；默认 ULMO/LZMA(macOS 10.15+)
#   Engine 随主程序源码编译打包；不导入外部 Runtime 二进制。
#   bun run build:mac --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 未知 --xxx 选项会原样透传给 tauri build
#   bun run build:mac arm --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 架构参数仍放在 tauri build 参数之前
#
# 架构别名:
#   arm / aarch64 / silicon  -> aarch64-apple-darwin   (Apple Silicon 原生)
#   intel / x64 / x86_64    -> x86_64-apple-darwin    (Intel)
#   分别构建 ARM 和 Intel 原生产物；Intel 包含独立的 ONNX Runtime。
#
# 缺失的 Rust 编译目标会自动 `rustup target add`。
#
# 签名(--signed)说明:
#   密钥/口令全部来自本地 apps/desktop/signing/.env.signing(已 gitignore,绝不入库),
#   与 build:signed 用同一份配置，并要求钥匙串中已安装 APPLE_SIGNING_IDENTITY。
#   签名最终 App，按配置公证/staple；DMG/updater 均从该 App 生成。
#
# 注:Windows / Linux 包无法在 macOS 上交叉构建,请到对应系统上分别用
#     bun run build:win / build:linux。
# ============================================================================
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "❌ build:mac 只能在 macOS 上运行(当前: $(uname -s))。" >&2
  echo "   Windows 包用 build:win,Linux 包用 build:linux,且都需在对应系统上构建。" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"
CONF="apps/desktop/tauri.conf.json"
MAC_CONF="apps/desktop/tauri.macos.conf.json"
DIST="$ROOT/dist/desktop"
RELEASE_LOCK_TOOL="$ROOT/scripts/release/release-lock.mjs"
APP_BUNDLE_TOOL="$ROOT/scripts/lib/macos-app-bundle.mjs"
ONNX_RUNTIME_TOOL="$ROOT/scripts/lib/macos-onnx-runtime.mjs"
DMG_TOOL="$ROOT/scripts/lib/macos-dmg.mjs"
CHECK_ONLY=0

# ── 解析参数:架构选择/开关归本脚本,未知 --xxx 起原样透传给 tauri build ─────
SELECT=()
PASSTHRU=()
SIGNED=0
seen_dashdash=0
for arg in "$@"; do
  # Reject retired options even after --; never forward them to Tauri.
  if [[ "$arg" == "--debug" || "$arg" == "-d" || "$arg" == "--no-bundle" || "$arg" == "--no-sign" || "$arg" == "--target" || "$arg" == --target=* || "$arg" == "-t" || "$arg" == "--profile" || "$arg" == --profile=* || "$arg" == "--bundles" || "$arg" == --bundles=* || "$arg" == "-b" ]]; then
    echo "❌ build:mac 固定生成完整 macOS release App/DMG；不能透传 ${arg}。开发包请使用 build:fast。" >&2
    exit 1
  elif [[ "$arg" == "--signed" ]]; then
    SIGNED=1
  elif [[ "$arg" == "--check" || "$arg" == "--check-only" ]]; then
    CHECK_ONLY=1
  elif [[ "$seen_dashdash" -eq 1 ]]; then
    PASSTHRU+=("$arg")
  elif [[ "$arg" == "--" ]]; then
    seen_dashdash=1
  elif [[ "$arg" == -* ]]; then
    PASSTHRU+=("$arg")
    seen_dashdash=1
  else
    SELECT+=("$arg")
  fi
done

# Validate the installation-container choice before any target installation or
# compilation. Both native targets use ULMO, supported on macOS 10.15+.
DMG_FORMAT="$(bun "$DMG_TOOL" format)"

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "❌ macOS packaging requires '$1'." >&2
    exit 1
  }
}

for tool in bun cargo git rustup lipo hdiutil ditto codesign tar cmp; do
  require_tool "$tool"
done

[[ -f "$RELEASE_LOCK_TOOL" ]] || {
  echo "❌ missing release-lock tool: $RELEASE_LOCK_TOOL" >&2
  exit 1
}
[[ -f "$ROOT/$MAC_CONF" ]] || {
  echo "❌ missing macOS Tauri overlay: $ROOT/$MAC_CONF" >&2
  exit 1
}


# 把别名规整成 rustc target triple
resolve_triple() {
  case "$1" in
    arm|aarch64|silicon|aarch64-apple-darwin)        echo "aarch64-apple-darwin" ;;
    intel|x64|x86_64|x86_64-apple-darwin)            echo "x86_64-apple-darwin" ;;
    universal|all-arch|universal-apple-darwin)       echo "universal-apple-darwin" ;;
    *) echo "❌ 未知架构: $1 (可选: arm / intel / universal)" >&2; exit 1 ;;
  esac
}

TRIPLES=()
if [[ "${#SELECT[@]}" -eq 0 ]]; then
  # 默认 Apple Silicon；Intel 包含匹配的 ONNX Runtime。
  TRIPLES=(aarch64-apple-darwin)
else
  for s in "${SELECT[@]}"; do
    TRIPLES+=("$(resolve_triple "$s")")
  done
fi

for t in "${TRIPLES[@]}"; do
  if [[ "$t" != "aarch64-apple-darwin" && "$t" != "x86_64-apple-darwin" ]]; then
    echo "❌ 请分别构建 arm 和 intel，不能生成 $t 包。" >&2
    exit 1
  fi
done

# ── 确保所需 Rust target 已安装(universal 需要底层两个 target 都在) ──────────
ensure_target() {
  local t="$1"
  if ! rustup target list --installed | grep -qx "$t"; then
    echo "▶ 安装 Rust target: $t"
    rustup target add "$t"
  fi
}
for t in "${TRIPLES[@]}"; do
  if [[ "$t" == "universal-apple-darwin" ]]; then
    ensure_target aarch64-apple-darwin
    ensure_target x86_64-apple-darwin
  else
    ensure_target "$t"
  fi
done


verify_macos_app() {
  local app="$1"
  local target="$2"
  local binary="$app/Contents/MacOS/nomifun-desktop"
  [[ -d "$app" && -f "$binary" ]] || {
    echo "❌ Tauri did not produce the expected macOS app: $app" >&2
    exit 1
  }
  local archs
  archs="$(lipo -archs "$binary" 2>/dev/null)" || {
    echo "❌ packaged app executable is not a valid Mach-O binary: $binary" >&2
    exit 1
  }
  if [[ "$target" == "universal-apple-darwin" ]]; then
    [[ "$archs" == *"arm64"* && "$archs" == *"x86_64"* ]] || {
      echo "❌ Universal app is missing arm64 or x86_64 slice: $archs" >&2
      exit 1
    }
  elif [[ "$target" == "aarch64-apple-darwin" ]]; then
    [[ "$archs" == "arm64" ]] || { echo "❌ arm64 app has architectures: $archs" >&2; exit 1; }
  else
    [[ "$archs" == "x86_64" ]] || { echo "❌ x86_64 app has architectures: $archs" >&2; exit 1; }
  fi
  bun "$APP_BUNDLE_TOOL" inspect --app "$app" >/dev/null
  if [[ "$target" == "x86_64-apple-darwin" ]]; then
    local ort_library="$app/Contents/Frameworks/$(bun -e 'console.log(require("./apps/desktop/onnx-runtime-intel.json").library)')"
    [[ -f "$ort_library" && "$(lipo -archs "$ort_library")" == "x86_64" ]] || {
      echo "❌ Intel App 缺少匹配的 ONNX Runtime: $ort_library" >&2
      exit 1
    }
  fi
}

notarize_final_app() {
  local app="$1"
  [[ "$SIGNED" -eq 1 && "$HAS_NOTARY" -eq 1 ]] || return 0
  local temporary
  temporary="$(mktemp -d "${TMPDIR:-/tmp}/nomifun-final-app-notary.XXXXXX")"
  if ! ditto -c -k --keepParent "$app" "$temporary/NomiFun.zip"; then
    rm -rf "$temporary"
    return 1
  fi
  echo "▶ 公证最终 App 并装订票据"
  if ! submit_for_notarization "$temporary/NomiFun.zip"; then
    rm -rf "$temporary"
    return 1
  fi
  rm -rf "$temporary"
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
}

sign_final_app() {
  local app="$1"
  local target="$2"
  local identity="-"
  if [[ "$SIGNED" -eq 1 ]]; then
    identity="${APPLE_SIGNING_IDENTITY:?installed signing identity is required}"
  fi
  echo "▶ 签名最终 App 和随附的原生库"
  bun "$APP_BUNDLE_TOOL" sign --app "$app" --identity "$identity" --target "$target"
}

create_dmg_from_staged_app() {
  local app="$1"
  local target="$2"
  local dmg_dir="$3"
  local version
  version="$(bun -e 'console.log(require("./package.json").version)')"
  local suffix="aarch64"
  [[ "$target" != "x86_64-apple-darwin" ]] || suffix="x64"
  local output="$dmg_dir/NomiFun_${version}_${suffix}.dmg"
  echo "▶ 从最终 App 生成 $DMG_FORMAT DMG: $output"
  NOMIFUN_MACOS_DMG_FORMAT="$DMG_FORMAT" bun "$DMG_TOOL" create --app "$app" --output "$output"
  if [[ "$SIGNED" -eq 1 ]]; then
    echo "▶ 签名 DMG: $output"
    codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$output"
    codesign --verify --strict --verbose=2 "$output"
  fi
}

write_release_lock() {
  local app="$1"
  local target="$2"
  local package="$3"
  local output="$4"
  local host="$app/Contents/MacOS/nomifun-desktop"
  local license="$app/Contents/Resources/LICENSE"
  local notice="$app/Contents/Resources/NOTICE"

  for artifact in "$host" "$package" "$license" "$notice"; do
    [[ -f "$artifact" && ! -L "$artifact" ]] || {
      echo "❌ cannot create release lock; real packaged artifact is missing or symlinked: $artifact" >&2
      exit 1
    }
  done

  local args=(
    "$RELEASE_LOCK_TOOL" create
    --root "$ROOT"
    --platform "$target"
    --host "$host"
    --package "$package"
    --legal "$license"
    --legal "$notice"
    --output "$output"
  )
  if [[ "$target" == "x86_64-apple-darwin" ]]; then
    args+=(--legal "$app/Contents/Resources/onnxruntime-LICENSE"
      --legal "$app/Contents/Resources/onnxruntime-ThirdPartyNotices.txt")
  fi

  echo "▶ 生成真实 release lock: $output"
  bun "${args[@]}" >/dev/null
  bun "$RELEASE_LOCK_TOOL" verify --root "$ROOT" --lock "$output" >/dev/null
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  echo "✅ macOS build tools and targets are ready; no app, DMG, release lock, or Engine execution was verified."
  exit 0
fi

# ── 签名:加载本地密钥并做基本校验(公共库,与 build:signed 共用一份实现) ────────
# shellcheck source=lib/mac-signing.sh
source "$SCRIPT_DIR/lib/mac-signing.sh"

HAS_NOTARY=0
if [[ "$SIGNED" -eq 1 ]]; then
  load_signing_env "$ROOT"
  require_signing_identity
  [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]] || {
    echo "❌ 最终 App 签名需要已安装的 APPLE_SIGNING_IDENTITY；仅 APPLE_CERTIFICATE 的临时钥匙串不能供后续签名复用。" >&2
    exit 1
  }
  detect_notary

  echo "▶ 签名身份: ${APPLE_SIGNING_IDENTITY:-(用 .p12: APPLE_CERTIFICATE)}"
  [[ "$HAS_NOTARY" -eq 1 ]] && echo "▶ 公证: 已启用,每个 target 构建后自动提交 Apple 并 staple"
fi

mkdir -p "$DIST"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "将依次构建以下目标: ${TRIPLES[*]}"
[[ "$SIGNED" -eq 1 ]] && echo "签名: 开启 (公证: $([[ "$HAS_NOTARY" -eq 1 ]] && echo 开启 || echo 关闭))" || echo "签名: 关闭 (本地测试包)"
echo "Engine: 随主程序编译注册，不导入外部 Runtime 二进制"
echo "产物汇总目录: $DIST"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

COLLECTED=()
COLLECTED_LOCKS=()
BASE_RUSTFLAGS="${RUSTFLAGS:-}"
for t in "${TRIPLES[@]}"; do
  echo ""
  echo "▶▶▶ 构建 $t ..."
  # Tauri's updater archive precedes final signing. Disable it here and generate
  # both distribution formats only after the final App is signed.
  updater="$(bun "$APP_BUNDLE_TOOL" build-settings --root "$ROOT" -- ${PASSTHRU[@]+"${PASSTHRU[@]}"})"
  ONNX_CONFIG=()
  unset ORT_LIB_PATH ORT_LIB_LOCATION ORT_PREFER_DYNAMIC_LINK
  export RUSTFLAGS="$BASE_RUSTFLAGS"
  if [[ "$t" == "x86_64-apple-darwin" ]]; then
    echo "▶ 准备校验过的 Intel ONNX Runtime/API 23"
    export ORT_LIB_PATH="$(bun "$ONNX_RUNTIME_TOOL")"
    export ORT_PREFER_DYNAMIC_LINK=1
    export RUSTFLAGS="$BASE_RUSTFLAGS -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"
    ONNX_CONFIG=(--config "$(dirname "$ORT_LIB_PATH")/tauri.conf.json")
  fi
  CI=true env -u APPLE_API_KEY -u APPLE_API_KEY_PATH -u APPLE_API_ISSUER \
    -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
    bun x tauri build --config "$CONF" --config "$MAC_CONF" \
    --target "$t" ${PASSTHRU[@]+"${PASSTHRU[@]}"} ${ONNX_CONFIG[@]+"${ONNX_CONFIG[@]}"} --no-sign \
    --config '{"bundle":{"active":true,"targets":["app"],"createUpdaterArtifacts":false}}'

  # tauri 把 DMG 放在 target/<triple>/release/bundle/dmg/*.dmg
  dmg_dir="$ROOT/target/$t/release/bundle/dmg"
  app="$ROOT/target/$t/release/bundle/macos/NomiFun.app"
  # Tauri's intermediate App is unsigned, so the copied executable must equal
  # this exact Cargo output before final nested signing changes its seal.
  cmp -s "$ROOT/target/$t/release/nomifun-desktop" "$app/Contents/MacOS/nomifun-desktop" || {
    echo "❌ Tauri App does not contain this exact desktop build; refusing stale or mismatched output。" >&2
    exit 1
  }
  sign_final_app "$app" "$t"
  verify_macos_app "$app" "$t"
  notarize_final_app "$app"
  create_dmg_from_staged_app "$app" "$t" "$dmg_dir"
  if [[ "$updater" == "true" ]]; then
    echo "▶ 从同一最终 App 生成并签名 updater"
    bun "$APP_BUNDLE_TOOL" updater --root "$ROOT" --app "$app"
    archive_name="NomiFun.app.tar.gz"
    [[ "$t" != "x86_64-apple-darwin" ]] || archive_name="NomiFun_$(bun -e 'console.log(require("./package.json").version)')_x64.app.tar.gz"
    archive="$app.tar.gz"
    if [[ "$t" == "x86_64-apple-darwin" ]]; then
      archive="$(dirname "$app")/$archive_name"
      mv -f "$app.tar.gz" "$archive"
      mv -f "$app.tar.gz.sig" "$archive.sig"
    fi
    # 下一架构的 pre-build 会清除 target bundle，先保存已验证的 updater。
    cp -f "$archive" "$DIST/$archive_name"
    cp -f "$archive.sig" "$DIST/$archive_name.sig"
  else
    # A prior updater is not valid evidence for this new build.
    rm -f "$app.tar.gz" "$app.tar.gz.sig"
  fi

  # 先公证(staple 会原地改写 DMG),再拷贝到汇总目录,保证收的是带票据的包
  notarize_dmg_dir "$dmg_dir"

  while IFS= read -r -d '' dmg; do
    package="$DIST/$(basename "$dmg")"
    lock="${package%.dmg}.release-lock.json"
    cp -f "$dmg" "$package"
    write_release_lock "$app" "$t" "$package" "$lock"
    COLLECTED+=("$package")
    COLLECTED_LOCKS+=("$lock")
  done < <(find "$dmg_dir" -maxdepth 1 -type f -name "NomiFun_$(bun -e 'console.log(require("./package.json").version)')_*.dmg" -print0 2>/dev/null)
done

# COLLECTED 为空 = 这一轮没产出任何 DMG(多半 bundle.targets 不含 dmg)。
# bash 3.2 下 `set -u` 还会让空数组展开直接报 unbound variable,所以这里既兜底
# 又把真正的失败原因说清楚,而不是假装「✅ 全部完成」。
if [[ "${#COLLECTED[@]}" -eq 0 ]]; then
  echo "" >&2
  echo "❌ 没有收集到任何 DMG —— tauri build 没在 macOS 上产出安装包。" >&2
  echo "   回看上面 tauri build 的输出:应出现 Bundling …dmg / Finished N bundles。" >&2
  echo "   若没有,多半是 bundle.targets 不含 dmg(本脚本本应已用 --config 覆盖)。" >&2
  exit 1
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 全部完成,DMG 已汇总到 $DIST :"
for f in "${COLLECTED[@]}"; do
  size="$(du -h "$f" | cut -f1)"
  printf "   %-40s %s\n" "$(basename "$f")" "$size"
done
echo "Release locks:"
for f in "${COLLECTED_LOCKS[@]}"; do
  printf "   %s\n" "$(basename "$f")"
done
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
