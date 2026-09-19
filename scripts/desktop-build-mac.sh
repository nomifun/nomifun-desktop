#!/usr/bin/env bash
# ============================================================================
# 打 macOS 桌面端安装包(.dmg),并汇总到 dist/desktop/。仅能在 macOS 上运行。
#
#   bun run build:mac                 # 默认打 Apple Silicon arm64 DMG(不签名)
#   bun run build:mac --signed        # arm64 + Developer ID 签名 + 公证
#   bun run build:mac arm             # 显式选择 Apple Silicon
#   Engine 随主程序源码编译打包；不导入外部 Runtime 二进制。
#   bun run build:mac --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 未知 --xxx 选项会原样透传给 tauri build
#   bun run build:mac arm --config '{"bundle":{"createUpdaterArtifacts":true}}'
#                                     # 架构参数仍放在 tauri build 参数之前
#
# 架构别名:
#   arm / aarch64 / silicon  -> aarch64-apple-darwin   (Apple Silicon 原生)
#   intel / x64 / x86_64     -> x86_64-apple-darwin     (Intel 原生 / M 系 Rosetta)
#   universal / all-arch     -> universal-apple-darwin  (二合一胖包,通吃两种 Mac)
#
# 缺失的 Rust 编译目标会自动 `rustup target add`。
#
# 签名(--signed)说明:
#   密钥/口令全部来自本地 apps/desktop/signing/.env.signing(已 gitignore,绝不入库),
#   与 build:signed 用同一份配置。签名在 tauri build 阶段由环境变量自动完成,公证在
#   每个 target 构建结束后对其 DMG 逐个提交 Apple 并 staple。文件不存在时直接报错。
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
CEF_STAGE_TOOL="$ROOT/scripts/validation/stage-macos-cef-bundle.mjs"
CHECK_ONLY=0

# ── 解析参数:架构选择/开关归本脚本,未知 --xxx 起原样透传给 tauri build ─────
SELECT=()
PASSTHRU=()
SIGNED=0
seen_dashdash=0
for arg in "$@"; do
  # Reject retired options even after --; never forward them to Tauri.
  if [[ "$arg" == "--with-codex-runtime" || "$arg" == --with-codex-runtime=* ]]; then
    echo "❌ 外部 Codex Runtime 打包入口已移除；Engine 必须随主程序源码编译注册。" >&2
    exit 1
  elif [[ "$seen_dashdash" -eq 1 ]]; then
    PASSTHRU+=("$arg")
  elif [[ "$arg" == "--" ]]; then
    seen_dashdash=1
  elif [[ "$arg" == "--signed" ]]; then
    SIGNED=1
  elif [[ "$arg" == "--check" || "$arg" == "--check-only" ]]; then
    CHECK_ONLY=1
  elif [[ "$arg" == --* ]]; then
    PASSTHRU+=("$arg")
    seen_dashdash=1
  else
    SELECT+=("$arg")
  fi
done

require_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "❌ macOS packaging requires '$1'." >&2
    exit 1
  }
}

for tool in bun cargo git rustup lipo hdiutil ditto codesign; do
  require_tool "$tool"
done

[[ -f "$RELEASE_LOCK_TOOL" ]] || {
  echo "❌ missing release-lock tool: $RELEASE_LOCK_TOOL" >&2
  exit 1
}
[[ -f "$CEF_STAGE_TOOL" ]] || {
  echo "❌ missing CEF staging tool: $CEF_STAGE_TOOL" >&2
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
  # Managed Browser 使用固定的 macOS arm64 CEF runtime；不得生成缺少
  # Browser framework/helper 的伪 Universal/Intel 包。
  TRIPLES=(aarch64-apple-darwin)
else
  for s in "${SELECT[@]}"; do
    TRIPLES+=("$(resolve_triple "$s")")
  done
fi

for t in "${TRIPLES[@]}"; do
  if [[ "$t" != "aarch64-apple-darwin" ]]; then
    echo "❌ 当前固定 CEF runtime 仅支持 Apple Silicon arm64；不能生成不完整的 $t 包。" >&2
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
  local retired_artifact
  # Include symlinks and hello metadata. A failed traversal is not evidence
  # that the retired resource is absent.
  [[ -d "$app/Contents/Resources" && ! -L "$app/Contents/Resources" ]] || {
    echo "❌ app Resources 目录缺失或为符号链接: $app" >&2
    exit 1
  }
  retired_artifact="$(find "$app/Contents/Resources" \
    \( -name 'nomifun-codex-runtime' -o -name 'nomifun-codex-runtime.hello.json' \) \
    -print -quit)" || {
    echo "❌ 无法检查 app 中的已退役 Runtime 资源: $app" >&2
    exit 1
  }
  if [[ -n "$retired_artifact" ]]; then
    echo "❌ app 包含已退役 Codex Runtime 资源: $retired_artifact" >&2
    exit 1
  fi

  local cef_framework="$app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework"
  local cef_runtime="$app/Contents/Resources/browser-cef/runtime.json"
  [[ -f "$cef_framework" && -f "$cef_runtime" ]] || {
    echo "❌ app 缺少固定 CEF framework/runtime metadata: $app" >&2
    exit 1
  }
  local helper_name
  for helper_name in \
    "NomiFun Helper" \
    "NomiFun Helper (GPU)" \
    "NomiFun Helper (Renderer)" \
    "NomiFun Helper (Plugin)" \
    "NomiFun Helper (Alerts)"; do
    [[ -x "$app/Contents/Frameworks/$helper_name.app/Contents/MacOS/$helper_name" ]] || {
      echo "❌ app 缺少可执行 CEF helper: $helper_name" >&2
      exit 1
    }
  done
}

find_cef_runtime() {
  local target="$1"
  local build_root="$ROOT/build.noindex/$target/release/build"
  local newest=""
  local newest_mtime=0
  local archive mtime
  while IFS= read -r -d '' archive; do
    mtime="$(stat -f '%m' "$archive")"
    if [[ -z "$newest" || "$mtime" -gt "$newest_mtime" ]]; then
      newest="$archive"
      newest_mtime="$mtime"
    fi
  done < <(find "$build_root" -path '*/out/cef_macos_aarch64/archive.json' -type f -print0 2>/dev/null)
  [[ -n "$newest" ]] || {
    echo "❌ cargo 未生成固定的 macOS arm64 CEF runtime。" >&2
    exit 1
  }
  dirname "$newest"
}

stage_macos_cef() {
  local app="$1"
  local target="$2"
  local identity="-"
  if [[ "$SIGNED" -eq 1 ]]; then
    identity="${APPLE_SIGNING_IDENTITY:-}"
    [[ -n "$identity" ]] || {
      echo "❌ CEF nested signing requires APPLE_SIGNING_IDENTITY。" >&2
      exit 1
    }
  fi

  echo "▶ 构建固定 CEF helper: $target"
  cargo build --locked -p nomifun-browser-macos --bin nomifun-browser-cef-helper \
    --release --target "$target"
  local helper="$ROOT/target/$target/release/nomifun-browser-cef-helper"
  local runtime
  runtime="$(find_cef_runtime "$target")"
  echo "▶ 装配并签名固定 CEF runtime/helper"
  bun "$CEF_STAGE_TOOL" --app "$app" --helper "$helper" --runtime "$runtime" --identity "$identity"
}

create_dmg_from_staged_app() {
  local app="$1"
  local target="$2"
  local dmg_dir="$3"
  local version
  version="$(bun -e 'console.log(require("./package.json").version)')"
  local output="$dmg_dir/NomiFun_${version}_aarch64.dmg"
  local temporary
  temporary="$(mktemp -d "${TMPDIR:-/tmp}/nomifun-arm64-dmg.XXXXXX")"
  local staging="$temporary/root"
  mkdir -p "$staging" "$dmg_dir"
  if ! ditto --noqtn "$app" "$staging/NomiFun.app"; then
    rm -rf "$temporary"
    return 1
  fi
  ln -s /Applications "$staging/Applications"
  echo "▶ 从已装配 CEF 的 App 生成 DMG: $output"
  if ! hdiutil create -quiet -ov -fs HFS+ -format UDZO \
    -volname NomiFun -srcfolder "$staging" "$temporary/NomiFun.dmg"; then
    rm -rf "$temporary"
    return 1
  fi
  mv -f "$temporary/NomiFun.dmg" "$output"
  rm -rf "$temporary"
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
for t in "${TRIPLES[@]}"; do
  echo ""
  echo "▶▶▶ 构建 $t ..."
  # 先只生成 App；CEF 必须在创建 DMG 前装入并完成 nested signing。
  CI=true bun x tauri build --config "$CONF" --config "$MAC_CONF" \
    --config '{"bundle":{"targets":["app"]}}' \
    --target "$t" ${PASSTHRU[@]+"${PASSTHRU[@]}"}

  # tauri 把 DMG 放在 target/<triple>/release/bundle/dmg/*.dmg
  dmg_dir="$ROOT/target/$t/release/bundle/dmg"
  app="$ROOT/target/$t/release/bundle/macos/NomiFun.app"
  stage_macos_cef "$app" "$t"
  verify_macos_app "$app" "$t"
  create_dmg_from_staged_app "$app" "$t" "$dmg_dir"

  # 先公证(staple 会原地改写 DMG),再拷贝到汇总目录,保证收的是带票据的包
  notarize_dmg_dir "$dmg_dir"

  while IFS= read -r -d '' dmg; do
    package="$DIST/$(basename "$dmg")"
    lock="${package%.dmg}.release-lock.json"
    cp -f "$dmg" "$package"
    write_release_lock "$app" "$t" "$package" "$lock"
    COLLECTED+=("$package")
    COLLECTED_LOCKS+=("$lock")
  done < <(find "$dmg_dir" -maxdepth 1 -type f -name '*.dmg' -print0 2>/dev/null)
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
